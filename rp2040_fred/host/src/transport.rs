use std::io;
use std::time::{Duration, Instant};

use rp2040_fred_protocol::bridge_proto::{
    crc32_ieee, unpack_trace_sample, MsgType, Packet, CRC_SIZE, HEADER_SIZE, MIN_PACKET_SIZE,
    PACKET_MAGIC, PACKET_SIZE, PAYLOAD_SIZE, PROTOCOL_VERSION, TRACE_PACKED_SAMPLE_SIZE,
};
use rusb::{Context, DeviceHandle, Direction, Error as UsbError, TransferType, UsbContext};

const LEGACY_PROTOCOL_VERSION: u8 = 1;
const LEGACY_PACKET_SIZE: usize = 32;
const LEGACY_PAYLOAD_SIZE: usize = 20;
const V2_PROTOCOL_VERSION: u8 = 2;
const V2_PACKET_SIZE: usize = 64;
const V2_PAYLOAD_SIZE: usize = V2_PACKET_SIZE - HEADER_SIZE - CRC_SIZE;
const V3_PROTOCOL_VERSION: u8 = 3;
const V3_TRACE_METADATA_SIZE: usize = 8;

pub trait HostTransport {
    fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>>;
}

pub struct UsbTransport {
    _ctx: Context,
    handle: DeviceHandle<Context>,
    in_ep: u8,
    out_ep: u8,
    timeout: Duration,
    warned_legacy_packets: bool,
}

impl UsbTransport {
    pub fn open(vid: u16, pid: u16) -> io::Result<Self> {
        let ctx = Context::new().map_err(io_other)?;
        let devices = ctx.devices().map_err(io_other)?;

        for device in devices.iter() {
            let desc = device.device_descriptor().map_err(io_other)?;
            if desc.vendor_id() != vid || desc.product_id() != pid {
                continue;
            }

            let config = device.active_config_descriptor().map_err(io_other)?;
            let mut in_ep = None;
            let mut out_ep = None;
            let mut if_num = None;

            for interface in config.interfaces() {
                for iface_desc in interface.descriptors() {
                    let candidate_if = iface_desc.interface_number();
                    let mut candidate_in = None;
                    let mut candidate_out = None;

                    for ep in iface_desc.endpoint_descriptors() {
                        if ep.transfer_type() != TransferType::Bulk {
                            continue;
                        }
                        match ep.direction() {
                            Direction::In => candidate_in = Some(ep.address()),
                            Direction::Out => candidate_out = Some(ep.address()),
                        }
                    }

                    if let (Some(i), Some(o)) = (candidate_in, candidate_out) {
                        if_num = Some(candidate_if);
                        in_ep = Some(i);
                        out_ep = Some(o);
                        break;
                    }
                }
                if if_num.is_some() {
                    break;
                }
            }

            let (if_num, in_ep, out_ep) = match (if_num, in_ep, out_ep) {
                (Some(if_num), Some(i), Some(o)) => (if_num, i, o),
                _ => continue,
            };

            let handle = device.open().map_err(io_other)?;
            handle
                .set_auto_detach_kernel_driver(true)
                .map_err(|_e| io::Error::new(io::ErrorKind::Unsupported, "auto detach"))?;
            handle.claim_interface(if_num).map_err(io_other)?;

            return Ok(Self {
                _ctx: ctx,
                handle,
                in_ep,
                out_ep,
                timeout: Duration::from_millis(600_000),
                warned_legacy_packets: false,
            });
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "USB device with matching VID/PID/interface not found",
        ))
    }

    pub fn read_packet(&mut self) -> io::Result<Packet> {
        self.read_packet_timeout(self.timeout)
    }

    pub fn read_packet_timeout(&mut self, timeout: Duration) -> io::Result<Packet> {
        loop {
            let mut buf = [0u8; PACKET_SIZE];
            let n = self
                .handle
                .read_bulk(self.in_ep, &mut buf, timeout)
                .map_err(io_other)?;

            // Embassy's CMSIS-DAP v2 class appends a zero-length packet after
            // full-size endpoint writes. Skip those framing packets.
            if n == 0 {
                eprintln!("read zero-length packet");
                continue;
            }

            let raw = &buf[..n];
            if raw.len() >= MIN_PACKET_SIZE && raw[1] == PROTOCOL_VERSION {
                return Packet::decode(raw).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("decode error: {:?}", e))
                });
            }

            if raw.len() >= MIN_PACKET_SIZE && raw[1] == V3_PROTOCOL_VERSION {
                if !self.warned_legacy_packets {
                    eprintln!(
                        "warning: device returned legacy protocol v3 packets; trace timestamps unavailable until firmware is upgraded"
                    );
                    self.warned_legacy_packets = true;
                }
                return decode_v3_packet(raw);
            }

            if n == V2_PACKET_SIZE && raw[1] == V2_PROTOCOL_VERSION {
                if !self.warned_legacy_packets {
                    eprintln!(
                        "warning: device returned legacy fixed-size 64-byte packets; likely old firmware/protocol v2"
                    );
                    self.warned_legacy_packets = true;
                }
                return decode_v2_packet(raw);
            }

            if n == LEGACY_PACKET_SIZE && raw[1] == LEGACY_PROTOCOL_VERSION {
                if !self.warned_legacy_packets {
                    eprintln!(
                        "warning: device returned legacy 32-byte packets; likely old firmware/protocol v1"
                    );
                    self.warned_legacy_packets = true;
                }
                return decode_legacy_packet(raw);
            }

            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unexpected USB packet size: got {n} bytes, expected a protocol v{PROTOCOL_VERSION} or v{V3_PROTOCOL_VERSION} packet between {MIN_PACKET_SIZE} and {PACKET_SIZE} bytes, a protocol v{V2_PROTOCOL_VERSION} packet of {V2_PACKET_SIZE} bytes, or a protocol v{LEGACY_PROTOCOL_VERSION} packet of {LEGACY_PACKET_SIZE} bytes"
                ),
            ));
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    fn write_packet(&mut self, pkt: &Packet) -> io::Result<()> {
        let raw = pkt.encode();
        let expected = pkt.encoded_len();
        let n = self
            .handle
            .write_bulk(self.out_ep, &raw[..expected], self.timeout)
            .map_err(io_other)?;
        if n != expected {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("short USB bulk write: got {n} bytes, expected {expected} bytes"),
            ));
        }
        Ok(())
    }
}

impl HostTransport for UsbTransport {
    fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>> {
        self.write_packet(&req)?;

        let deadline = Instant::now() + Duration::from_millis(5000);
        let mut replies = Vec::new();
        let want_seq = req.seq;

        while Instant::now() < deadline {
            match self.read_packet() {
                Ok(pkt) => {
                    let done = matches!(
                        pkt.msg_type,
                        rp2040_fred_protocol::bridge_proto::MsgType::Ack
                            | rp2040_fred_protocol::bridge_proto::MsgType::Nack
                    ) && pkt.seq == want_seq;
                    replies.push(pkt);
                    if done {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::TimedOut => continue,
                Err(e) => return Err(e),
            }
        }

        if replies.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "no USB response packet received",
            ));
        }

        Ok(replies)
    }
}

fn io_other(e: UsbError) -> io::Error {
    let kind = match e {
        UsbError::Timeout => io::ErrorKind::TimedOut,
        UsbError::NoDevice => io::ErrorKind::NotConnected,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, e.to_string())
}

fn decode_v3_packet(raw: &[u8]) -> io::Result<Packet> {
    if raw.len() < MIN_PACKET_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "v3 packet too short",
        ));
    }

    if raw[0] != PACKET_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v3 bad magic: 0x{:02X}", raw[0]),
        ));
    }
    if raw[1] != V3_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v3 bad protocol version: {}", raw[1]),
        ));
    }

    let payload_len = u16::from_le_bytes([raw[6], raw[7]]) as usize;
    if payload_len > PAYLOAD_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v3 invalid payload length: {payload_len}"),
        ));
    }

    let encoded_len = HEADER_SIZE + payload_len + CRC_SIZE;
    if raw.len() != encoded_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "v3 packet length mismatch: got {}, expected {encoded_len}",
                raw.len()
            ),
        ));
    }

    let msg_type = MsgType::from_u8(raw[2]).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v3 unknown msg type: 0x{:02X}", raw[2]),
        )
    })?;
    let crc_offset = HEADER_SIZE + payload_len;
    let expected_crc = u32::from_le_bytes([
        raw[crc_offset],
        raw[crc_offset + 1],
        raw[crc_offset + 2],
        raw[crc_offset + 3],
    ]);
    let actual_crc = crc32_ieee(&raw[..crc_offset]);
    if expected_crc != actual_crc {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "v3 CRC mismatch",
        ));
    }

    let seq = u16::from_le_bytes([raw[4], raw[5]]);
    let payload = &raw[HEADER_SIZE..crc_offset];

    if msg_type == MsgType::TraceSample {
        return decode_v3_trace_packet(seq, payload);
    }

    Packet::new(msg_type, seq, payload).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "v3 packet could not be converted to current packet format",
        )
    })
}

fn decode_legacy_packet(raw: &[u8]) -> io::Result<Packet> {
    if raw.len() != LEGACY_PACKET_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "legacy packet decode with wrong length",
        ));
    }

    if raw[0] != 0xA5 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("legacy bad magic: 0x{:02X}", raw[0]),
        ));
    }
    if raw[1] != LEGACY_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("legacy bad protocol version: {}", raw[1]),
        ));
    }

    let payload_len = raw[3] as usize;
    if payload_len > LEGACY_PAYLOAD_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("legacy invalid payload length: {payload_len}"),
        ));
    }

    let expected_crc = u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]);
    let actual_crc = crc32_ieee(&raw[..28]);
    if expected_crc != actual_crc {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "legacy CRC mismatch",
        ));
    }

    let msg_type = MsgType::from_u8(raw[2]).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("legacy unknown msg type: 0x{:02X}", raw[2]),
        )
    })?;
    let seq = u16::from_le_bytes([raw[4], raw[5]]);
    let payload = &raw[8..8 + payload_len];

    if msg_type == MsgType::TraceSample {
        let mut samples = Vec::new();
        for chunk in payload.chunks_exact(4) {
            samples.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        return Ok(Packet::trace_samples(seq, None, 0, 0, &samples));
    }

    Packet::new(msg_type, seq, payload).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "legacy packet could not be converted to current packet format",
        )
    })
}

fn decode_v2_packet(raw: &[u8]) -> io::Result<Packet> {
    if raw.len() != V2_PACKET_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "v2 packet decode with wrong length",
        ));
    }

    if raw[0] != 0xA5 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v2 bad magic: 0x{:02X}", raw[0]),
        ));
    }
    if raw[1] != V2_PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v2 bad protocol version: {}", raw[1]),
        ));
    }

    let payload_len = raw[3] as usize;
    if payload_len > V2_PAYLOAD_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v2 invalid payload length: {payload_len}"),
        ));
    }

    let expected_crc = u32::from_le_bytes([raw[60], raw[61], raw[62], raw[63]]);
    let actual_crc = crc32_ieee(&raw[..60]);
    if expected_crc != actual_crc {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "v2 CRC mismatch",
        ));
    }

    let msg_type = MsgType::from_u8(raw[2]).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("v2 unknown msg type: 0x{:02X}", raw[2]),
        )
    })?;
    let seq = u16::from_le_bytes([raw[4], raw[5]]);
    let payload = &raw[8..8 + payload_len];

    if msg_type == MsgType::TraceSample {
        let mut samples = Vec::new();
        for chunk in payload.chunks_exact(4) {
            samples.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        return Ok(Packet::trace_samples(seq, None, 0, 0, &samples));
    }

    Packet::new(msg_type, seq, payload).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "v2 packet could not be converted to current packet format",
        )
    })
}

fn decode_v3_trace_packet(seq: u16, payload: &[u8]) -> io::Result<Packet> {
    if payload.len() < V3_TRACE_METADATA_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "v3 trace payload too short",
        ));
    }

    let dropped_samples_total =
        u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let rx_stall_count_total = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    let sample_bytes = &payload[V3_TRACE_METADATA_SIZE..];
    if !sample_bytes.len().is_multiple_of(TRACE_PACKED_SAMPLE_SIZE) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "v3 trace payload sample bytes malformed",
        ));
    }

    let mut samples = Vec::with_capacity(sample_bytes.len() / TRACE_PACKED_SAMPLE_SIZE);
    for chunk in sample_bytes.chunks_exact(TRACE_PACKED_SAMPLE_SIZE) {
        samples.push(unpack_trace_sample([chunk[0], chunk[1], chunk[2]]));
    }

    Ok(Packet::trace_samples(
        seq,
        None,
        dropped_samples_total,
        rx_stall_count_total,
        &samples,
    ))
}

#[cfg(test)]
mod tests {
    use super::{decode_v3_packet, V3_PROTOCOL_VERSION};
    use rp2040_fred_protocol::bridge_proto::{
        crc32_ieee, pack_trace_sample, MsgType, CRC_SIZE, HEADER_SIZE, PACKET_MAGIC,
    };

    fn sample(data: u8, addr: u8, read: bool) -> u32 {
        (data as u32) | ((addr as u32) << 8) | ((read as u32) << 16) | (1 << 17)
    }

    fn encode_v3_trace_packet(
        seq: u16,
        dropped_samples_total: u32,
        rx_stall_count_total: u32,
        samples: &[u32],
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&dropped_samples_total.to_le_bytes());
        payload.extend_from_slice(&rx_stall_count_total.to_le_bytes());
        for sample in samples {
            payload.extend_from_slice(&pack_trace_sample(*sample));
        }

        let payload_len = payload.len() as u16;
        let mut raw = vec![0u8; HEADER_SIZE + payload.len() + CRC_SIZE];
        raw[0] = PACKET_MAGIC;
        raw[1] = V3_PROTOCOL_VERSION;
        raw[2] = MsgType::TraceSample as u8;
        raw[3] = 0;
        raw[4..6].copy_from_slice(&seq.to_le_bytes());
        raw[6..8].copy_from_slice(&payload_len.to_le_bytes());
        raw[HEADER_SIZE..HEADER_SIZE + payload.len()].copy_from_slice(&payload);
        let crc_offset = HEADER_SIZE + payload.len();
        let crc = crc32_ieee(&raw[..crc_offset]);
        raw[crc_offset..crc_offset + CRC_SIZE].copy_from_slice(&crc.to_le_bytes());
        raw
    }

    #[test]
    fn decode_v3_trace_packet_converts_missing_timestamp_to_none() {
        let raw = encode_v3_trace_packet(
            0x33,
            7,
            2,
            &[sample(0x04, 0x03, false), sample(0x5A, 0xA5, true)],
        );

        let pkt = decode_v3_packet(&raw).expect("decode");
        let trace = pkt.decode_trace_samples().expect("trace");
        assert_eq!(trace.timestamp_us, None);
        assert_eq!(trace.dropped_samples_total, 7);
        assert_eq!(trace.rx_stall_count_total, 2);
        let samples: Vec<_> = trace.iter_samples().collect();
        assert_eq!(
            samples,
            vec![sample(0x04, 0x03, false), sample(0x5A, 0xA5, true)]
        );
    }
}
