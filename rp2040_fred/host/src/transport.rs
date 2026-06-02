use std::io;
use std::time::{Duration, Instant};

use rp2040_fred_protocol::bridge_proto::{
    Packet, MIN_PACKET_SIZE, PACKET_SIZE, PROTOCOL_VERSION, USB_PROTOCOL_CAPTURE,
    USB_PROTOCOL_MASTER, USB_VENDOR_CLASS, USB_VENDOR_SUBCLASS,
};
use rusb::{Context, DeviceHandle, Direction, Error as UsbError, TransferType, UsbContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbRole {
    Master,
    Capture,
}

impl UsbRole {
    fn protocol(self) -> u8 {
        match self {
            Self::Master => USB_PROTOCOL_MASTER,
            Self::Capture => USB_PROTOCOL_CAPTURE,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Capture => "capture",
        }
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
    pub fn open(vid: u16, pid: u16, role: UsbRole) -> io::Result<Self> {
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
                    if !role_matches(
                        role,
                        iface_desc.class_code(),
                        iface_desc.sub_class_code(),
                        iface_desc.protocol_code(),
                    ) {
                        continue;
                    }

                    if let Some((i, o)) = select_bulk_pair(
                        iface_desc
                            .endpoint_descriptors()
                            .map(|ep| (ep.transfer_type(), ep.direction(), ep.address())),
                    ) {
                        if_num = Some(iface_desc.interface_number());
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
            });
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "USB device with matching VID/PID/{} interface not found",
                role.label()
            ),
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

            // Embassy sends a zero-length packet after full-size endpoint writes.
            // Skip those framing packets.
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

            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unexpected USB packet size: got {n} bytes, expected a protocol v{PROTOCOL_VERSION} packet between {MIN_PACKET_SIZE} and {PACKET_SIZE} bytes"
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

    pub fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>> {
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

fn role_matches(role: UsbRole, class_code: u8, sub_class_code: u8, protocol_code: u8) -> bool {
    class_code == USB_VENDOR_CLASS
        && sub_class_code == USB_VENDOR_SUBCLASS
        && protocol_code == role.protocol()
}

fn select_bulk_pair(
    endpoints: impl IntoIterator<Item = (TransferType, Direction, u8)>,
) -> Option<(u8, u8)> {
    let mut in_ep = None;
    let mut out_ep = None;

    for (transfer_type, direction, address) in endpoints {
        if transfer_type != TransferType::Bulk {
            continue;
        }
        match direction {
            Direction::In => in_ep = Some(address),
            Direction::Out => out_ep = Some(address),
        }
    }

    match (in_ep, out_ep) {
        (Some(i), Some(o)) => Some((i, o)),
        _ => None,
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

#[cfg(test)]
mod tests {
    use rusb::{Direction, TransferType};

    use super::{role_matches, select_bulk_pair, UsbRole};

    #[test]
    fn role_match_uses_vendor_class_and_protocol() {
        assert!(role_matches(UsbRole::Master, 0xFF, 0, 0x01));
        assert!(role_matches(UsbRole::Capture, 0xFF, 0, 0x02));
        assert!(!role_matches(UsbRole::Master, 0xFF, 0, 0x02));
        assert!(!role_matches(UsbRole::Capture, 0xFE, 0, 0x02));
    }

    #[test]
    fn bulk_pair_requires_in_and_out() {
        assert_eq!(
            select_bulk_pair([
                (TransferType::Interrupt, Direction::In, 0x83),
                (TransferType::Bulk, Direction::Out, 0x01),
                (TransferType::Bulk, Direction::In, 0x82),
            ]),
            Some((0x82, 0x01))
        );
        assert_eq!(
            select_bulk_pair([(TransferType::Bulk, Direction::In, 0x82)]),
            None
        );
        assert_eq!(
            select_bulk_pair([(TransferType::Bulk, Direction::Out, 0x01)]),
            None
        );
    }
}
