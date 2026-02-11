use std::io;

use rp2040_fred_firmware::bridge_proto::{MsgType, Packet};
use rp2040_fred_firmware::mock_bus::{MockBusFrame, MockBusRunner};

pub trait HostTransport {
    fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>>;
}

pub struct MockTransport {
    sim: MockBusRunner,
    telemetry_enabled: bool,
}

impl MockTransport {
    pub const fn new() -> Self {
        Self {
            sim: MockBusRunner::new(),
            telemetry_enabled: false,
        }
    }

    pub fn next_frame(&mut self) -> Option<MockBusFrame> {
        if self.telemetry_enabled {
            Some(self.sim.step())
        } else {
            None
        }
    }
}

impl HostTransport for MockTransport {
    fn transact(&mut self, req: Packet) -> io::Result<Vec<Packet>> {
        let mut replies = Vec::new();
        match req.msg_type {
            MsgType::Ping => {
                replies.push(Packet::ack(req.seq, MsgType::Ping, 0));
            }
            MsgType::TelemetrySet => {
                if req.payload_len < 1 {
                    replies.push(Packet::nack(req.seq, MsgType::TelemetrySet as u8, 1));
                } else {
                    self.telemetry_enabled = req.payload[0] != 0;
                    replies.push(Packet::ack(req.seq, MsgType::TelemetrySet, 0));
                }
            }
            MsgType::SnapshotReq => {
                if let Some(frame) = self.next_frame() {
                    replies.push(Packet::telemetry(
                        req.seq,
                        0,
                        frame.response_fcf1 as i32,
                        0,
                        0,
                        self.telemetry_enabled as u8,
                    ));
                }
                replies.push(Packet::ack(req.seq, MsgType::SnapshotReq, 0));
            }
            _ => {
                replies.push(Packet::nack(req.seq, req.msg_type as u8, 0xFE));
            }
        }
        Ok(replies)
    }
}

pub struct UsbTransport;

impl UsbTransport {
    pub fn open(_vid: u16, _pid: u16, _if_num: u8) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "USB transport not yet implemented in this phase (protocol + mock mode only)",
        ))
    }
}

impl HostTransport for UsbTransport {
    fn transact(&mut self, _req: Packet) -> io::Result<Vec<Packet>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "USB transport not yet implemented",
        ))
    }
}
