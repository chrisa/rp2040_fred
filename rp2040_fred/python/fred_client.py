"""Python USB client for the RP2040 FRED bridge.

Dependency: pyusb (pip install pyusb)

This mirrors the bridge protocol used by the Rust host client.
"""

from __future__ import annotations

import binascii
import struct
from dataclasses import dataclass
from typing import Dict, Optional

import usb.core
import usb.util

PACKET_MAGIC = 0xA5
PROTOCOL_VERSION = 1
PACKET_SIZE = 32
PAYLOAD_SIZE = 20

MSG_PING = 0x01
MSG_TELEMETRY_SET = 0x10
MSG_SNAPSHOT_REQ = 0x12
MSG_ACK = 0x80
MSG_NACK = 0x81
MSG_TELEMETRY = 0x90


class FredProtocolError(RuntimeError):
    """Bridge protocol parse or transaction failure."""


class FredUsbError(RuntimeError):
    """USB transport error."""


@dataclass
class _Packet:
    msg_type: int
    seq: int
    payload: bytes

    def encode(self) -> bytes:
        if len(self.payload) > PAYLOAD_SIZE:
            raise FredProtocolError("payload too large")

        fixed_payload = self.payload.ljust(PAYLOAD_SIZE, b"\x00")
        header = struct.pack(
            "<BBBBH",
            PACKET_MAGIC,
            PROTOCOL_VERSION,
            self.msg_type,
            len(self.payload),
            self.seq & 0xFFFF,
        )
        reserved = b"\x00\x00"
        body = header + reserved + fixed_payload
        crc = binascii.crc32(body) & 0xFFFFFFFF
        return body + struct.pack("<I", crc)

    @staticmethod
    def decode(raw: bytes) -> "_Packet":
        if len(raw) < PACKET_SIZE:
            raise FredProtocolError(f"short packet: {len(raw)} bytes")

        raw = raw[:PACKET_SIZE]
        magic, version, msg_type, payload_len, seq = struct.unpack_from("<BBBBH", raw, 0)
        if magic != PACKET_MAGIC:
            raise FredProtocolError(f"bad magic: 0x{magic:02X}")
        if version != PROTOCOL_VERSION:
            raise FredProtocolError(f"bad protocol version: {version}")
        if payload_len > PAYLOAD_SIZE:
            raise FredProtocolError(f"invalid payload length: {payload_len}")

        expected_crc = struct.unpack_from("<I", raw, 28)[0]
        actual_crc = binascii.crc32(raw[:28]) & 0xFFFFFFFF
        if expected_crc != actual_crc:
            raise FredProtocolError("CRC mismatch")

        payload = raw[8 : 8 + payload_len]
        return _Packet(msg_type=msg_type, seq=seq, payload=payload)


class FredUsbClient:
    """RP2040 FRED USB client.

    Example:
        c = FredUsbClient(0x2E8A, 0x000A)
        c.enable_polling(period_ms=25)
        print(c.refresh())
    """

    def __init__(
        self,
        vid: int,
        pid: int,
        *,
        timeout_ms: int = 250,
        x_counts_per_mm: float = 100.0,
        z_counts_per_mm: float = 100.0,
    ) -> None:
        self.vid = vid
        self.pid = pid
        self.timeout_ms = timeout_ms
        self.x_counts_per_mm = x_counts_per_mm
        self.z_counts_per_mm = z_counts_per_mm

        self._dev: Optional[usb.core.Device] = None
        self._if_num: Optional[int] = None
        self._ep_in: Optional[int] = None
        self._ep_out: Optional[int] = None
        self._seq = 1

        self._latest = {
            "x_mm": 0.0,
            "z_mm": 0.0,
            "spindle_rpm": 0,
            "x_counts": 0,
            "z_counts": 0,
            "tick": 0,
            "flags": 0,
        }

        self._open()

    def close(self) -> None:
        if self._dev is None or self._if_num is None:
            return

        try:
            usb.util.release_interface(self._dev, self._if_num)
        except usb.core.USBError:
            pass

        try:
            self._dev.attach_kernel_driver(self._if_num)
        except (NotImplementedError, usb.core.USBError):
            pass

        usb.util.dispose_resources(self._dev)
        self._dev = None
        self._if_num = None
        self._ep_in = None
        self._ep_out = None

    def __del__(self) -> None:
        self.close()

    def enable_polling(self, period_ms: int = 25) -> None:
        payload = struct.pack("<BH", 1, int(period_ms) & 0xFFFF)
        self._transact(MSG_TELEMETRY_SET, payload)

    def disable_polling(self) -> None:
        payload = struct.pack("<BH", 0, 0)
        self._transact(MSG_TELEMETRY_SET, payload)

    def refresh(self) -> Dict[str, object]:
        """Drain pending USB telemetry and return latest snapshot.

        Returns a dict with keys:
          x_mm, z_mm, spindle_rpm, x_counts, z_counts, tick, flags
        """
        if self._dev is None:
            raise FredUsbError("device not open")

        while True:
            pkt = self._read_packet(timeout_ms=1)
            if pkt is None:
                break
            if pkt.msg_type == MSG_TELEMETRY and len(pkt.payload) >= 16:
                self._consume_telemetry(pkt)

        return dict(self._latest)

    def _open(self) -> None:
        dev = usb.core.find(idVendor=self.vid, idProduct=self.pid)
        if dev is None:
            raise FredUsbError(f"USB device {self.vid:04x}:{self.pid:04x} not found")

        dev.set_configuration()
        cfg = dev.get_active_configuration()

        chosen_intf = None
        chosen_in = None
        chosen_out = None

        for intf in cfg:
            bulk_in = None
            bulk_out = None
            for ep in intf:
                attrs = usb.util.endpoint_type(ep.bmAttributes)
                if attrs != usb.util.ENDPOINT_TYPE_BULK:
                    continue
                direction = usb.util.endpoint_direction(ep.bEndpointAddress)
                if direction == usb.util.ENDPOINT_IN:
                    bulk_in = ep.bEndpointAddress
                elif direction == usb.util.ENDPOINT_OUT:
                    bulk_out = ep.bEndpointAddress
            if bulk_in is not None and bulk_out is not None:
                chosen_intf = intf.bInterfaceNumber
                chosen_in = bulk_in
                chosen_out = bulk_out
                break

        if chosen_intf is None or chosen_in is None or chosen_out is None:
            raise FredUsbError("No USB interface with bulk IN+OUT endpoints found")

        if dev.is_kernel_driver_active(chosen_intf):
            try:
                dev.detach_kernel_driver(chosen_intf)
            except (NotImplementedError, usb.core.USBError):
                pass

        usb.util.claim_interface(dev, chosen_intf)

        self._dev = dev
        self._if_num = int(chosen_intf)
        self._ep_in = int(chosen_in)
        self._ep_out = int(chosen_out)

    def _next_seq(self) -> int:
        seq = self._seq & 0xFFFF
        self._seq = (self._seq + 1) & 0xFFFF
        return seq

    def _transact(self, msg_type: int, payload: bytes) -> None:
        seq = self._next_seq()
        req = _Packet(msg_type=msg_type, seq=seq, payload=payload)
        self._write_packet(req)

        got_ack = False
        # Read until matching ACK/NACK. Keep telemetry packets by decoding them into latest state.
        for _ in range(32):
            pkt = self._read_packet(timeout_ms=self.timeout_ms)
            if pkt is None:
                continue

            if pkt.msg_type == MSG_TELEMETRY and len(pkt.payload) >= 16:
                self._consume_telemetry(pkt)
                continue

            if pkt.seq != seq:
                continue

            if pkt.msg_type == MSG_ACK:
                got_ack = True
                break
            if pkt.msg_type == MSG_NACK:
                reason = pkt.payload[1] if len(pkt.payload) > 1 else 0xFF
                raise FredProtocolError(f"NACK for msg 0x{msg_type:02X}, reason 0x{reason:02X}")

        if not got_ack:
            raise FredUsbError("timeout waiting for ACK/NACK")

    def _consume_telemetry(self, pkt: _Packet) -> None:
        tick = struct.unpack_from("<I", pkt.payload, 0)[0]
        x_counts = struct.unpack_from("<i", pkt.payload, 4)[0]
        z_counts = struct.unpack_from("<i", pkt.payload, 8)[0]
        rpm = struct.unpack_from("<H", pkt.payload, 12)[0]
        flags = pkt.payload[14]

        self._latest.update(
            {
                "x_mm": (float(x_counts) * 2.0) / self.x_counts_per_mm,
                "z_mm": float(z_counts) / self.z_counts_per_mm,
                "spindle_rpm": int(rpm),
                "x_counts": int(x_counts),
                "z_counts": int(z_counts),
                "tick": int(tick),
                "flags": int(flags),
            }
        )

    def _write_packet(self, pkt: _Packet) -> None:
        if self._dev is None or self._ep_out is None:
            raise FredUsbError("device not open")
        raw = pkt.encode()
        written = self._dev.write(self._ep_out, raw, timeout=self.timeout_ms)
        if written != PACKET_SIZE:
            raise FredUsbError(f"short USB write: {written} bytes")

    def _read_packet(self, timeout_ms: int) -> Optional[_Packet]:
        if self._dev is None or self._ep_in is None:
            raise FredUsbError("device not open")

        try:
            data = self._dev.read(self._ep_in, 64, timeout=timeout_ms)
        except usb.core.USBError as exc:
            # pyusb maps timeout to backend-specific errno (often 110).
            if getattr(exc, "errno", None) in (110, 60) or "timed out" in str(exc).lower():
                return None
            raise FredUsbError(str(exc)) from exc

        raw = bytes(data)
        if len(raw) < PACKET_SIZE:
            raise FredProtocolError(f"unexpected USB packet size: {len(raw)}")
        return _Packet.decode(raw[:PACKET_SIZE])
