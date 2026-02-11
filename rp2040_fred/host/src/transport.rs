use std::io;
use std::time::{Duration, Instant};

use rp2040_fred_firmware::bridge_proto::{Packet, PACKET_SIZE};
use rp2040_fred_firmware::bridge_service::BridgeService;
use rusb::{Context, DeviceHandle, Direction, TransferType, UsbContext};

pub trait HostTransport {
    fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>>;
}

pub struct MockTransport {
    bridge: BridgeService,
}

impl MockTransport {
    pub const fn new() -> Self {
        Self {
            bridge: BridgeService::new(),
        }
    }

    pub fn next_packet(&mut self) -> Option<Packet> {
        self.bridge.poll_telemetry_event()
    }
}

impl HostTransport for MockTransport {
    fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>> {
        let mut out = [Packet::ping(0), Packet::ping(0)];
        let n = self.bridge.handle_request(req, &mut out);
        Ok(out[..n].to_vec())
    }
}

pub struct UsbTransport {
    _ctx: Context,
    handle: DeviceHandle<Context>,
    in_ep: u8,
    out_ep: u8,
    timeout: Duration,
}

impl UsbTransport {
    pub fn open(vid: u16, pid: u16, if_num: u8) -> io::Result<Self> {
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

            for interface in config.interfaces() {
                for iface_desc in interface.descriptors() {
                    if iface_desc.interface_number() != if_num {
                        continue;
                    }

                    for ep in iface_desc.endpoint_descriptors() {
                        if ep.transfer_type() != TransferType::Bulk {
                            continue;
                        }
                        match ep.direction() {
                            Direction::In => in_ep = Some(ep.address()),
                            Direction::Out => out_ep = Some(ep.address()),
                        }
                    }
                }
            }

            let (in_ep, out_ep) = match (in_ep, out_ep) {
                (Some(i), Some(o)) => (i, o),
                _ => continue,
            };

            let handle = device.open().map_err(io_other)?;
            let _ = handle.set_auto_detach_kernel_driver(true);
            handle.claim_interface(if_num).map_err(io_other)?;

            return Ok(Self {
                _ctx: ctx,
                handle,
                in_ep,
                out_ep,
                timeout: Duration::from_millis(250),
            });
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "USB device with matching VID/PID/interface not found",
        ))
    }

    pub fn read_packet(&mut self) -> io::Result<Packet> {
        let mut buf = [0u8; 64];
        let n = self
            .handle
            .read_bulk(self.in_ep, &mut buf, self.timeout)
            .map_err(io_other)?;

        if n != PACKET_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected USB packet size",
            ));
        }

        let mut raw = [0u8; PACKET_SIZE];
        raw.copy_from_slice(&buf[..PACKET_SIZE]);
        Packet::decode(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("decode error: {:?}", e)))
    }

    fn write_packet(&mut self, pkt: &Packet) -> io::Result<()> {
        let raw = pkt.encode();
        let n = self
            .handle
            .write_bulk(self.out_ep, &raw, self.timeout)
            .map_err(io_other)?;
        if n != PACKET_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short USB bulk write",
            ));
        }
        Ok(())
    }
}

impl HostTransport for UsbTransport {
    fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>> {
        self.write_packet(&req)?;

        let deadline = Instant::now() + Duration::from_millis(500);
        let mut replies = Vec::new();

        while Instant::now() < deadline {
            match self.read_packet() {
                Ok(pkt) => {
                    replies.push(pkt);
                    if !replies.is_empty() {
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

fn io_other<E: core::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{HostTransport, MockTransport};
    use rp2040_fred_firmware::bridge_proto::{MsgType, Packet};

    #[test]
    fn ping_ack_roundtrip() {
        let mut t = MockTransport::new();
        let r = t.transact(Packet::ping(42)).expect("transact");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].msg_type, MsgType::Ack);
        assert_eq!(r[0].seq, 42);
    }

    #[test]
    fn telemetry_enable_then_event() {
        let mut t = MockTransport::new();
        let _ = t
            .transact(Packet::telemetry_set(1, true, 50))
            .expect("telemetry_set");

        let mut seen = 0usize;
        for _ in 0..30 {
            if t.next_packet().is_some() {
                seen += 1;
            }
        }
        assert!(seen >= 1);
    }
}
