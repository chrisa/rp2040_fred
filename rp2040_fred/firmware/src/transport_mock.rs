use crate::resources::SnifferResources;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};
use rp2040_fred_protocol::bridge_service::BridgeService;

pub struct BridgeTransport {
    bridge: BridgeService,
    capture_enabled: bool,
}

impl BridgeTransport {
    pub fn new(_sniffer: SnifferResources) -> Self {
        Self {
            bridge: BridgeService::new(),
            capture_enabled: true,
        }
    }

    pub fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            MsgType::CaptureSet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::CaptureSet as u8, 1);
                } else {
                    self.capture_enabled = req.payload[0] != 0;
                    out[0] = Packet::ack(req.seq, MsgType::CaptureSet, 0);
                }
                1
            }
            _ => {
                if self.capture_enabled {
                    out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x10);
                    1
                } else {
                    self.bridge.handle_request(req, out)
                }
            }
        }
    }

    pub fn poll_outgoing_packet(&mut self) -> Option<Packet> {
        if self.capture_enabled {
            None
        } else {
            self.bridge.poll_telemetry_event()
        }
    }

    pub fn post_send_delay_ms(&self, pkt: &Packet) -> Option<u64> {
        if self.capture_enabled || pkt.msg_type != MsgType::Telemetry {
            None
        } else {
            Some(self.bridge.telemetry_period_ms().max(1) as u64)
        }
    }
}
