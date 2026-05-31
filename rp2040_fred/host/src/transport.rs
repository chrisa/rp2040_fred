use std::io;
use std::time::{Duration, Instant};

use rp2040_fred_protocol::bridge_proto::{
    Packet, MIN_PACKET_SIZE, PACKET_SIZE, PROTOCOL_VERSION
};
use rusb::{Context, DeviceHandle, Direction, Error as UsbError, TransferType, UsbContext};

pub struct UsbTransport {
    _ctx: Context,
    handle: DeviceHandle<Context>,
    in_ep: u8,
    out_ep: u8,
    timeout: Duration,
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

fn io_other(e: UsbError) -> io::Error {
    let kind = match e {
        UsbError::Timeout => io::ErrorKind::TimedOut,
        UsbError::NoDevice => io::ErrorKind::NotConnected,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, e.to_string())
}