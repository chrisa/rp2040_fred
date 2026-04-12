use crate::transport::Transport;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};

mod mock_bus;
mod bridge_service;
mod protocol;

use bridge_service::BridgeService;

pub struct MockTransport {
    bridge: BridgeService,
    capture_enabled: bool,
}

impl MockTransport {
    pub fn new() -> Self {
        Self {
            bridge: BridgeService::new(),
            capture_enabled: true,
        }
    }
}

impl Transport for MockTransport {

    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            _ => {
                self.bridge.handle_request(req, out)
            }
        }
    }

    fn poll_outgoing_packet(&mut self) -> Option<Packet> {
        self.bridge.poll_outgoing_packet()
    }

    fn post_send_delay_ms(&self, pkt: &Packet) -> Option<u64> {
        if self.capture_enabled || pkt.msg_type != MsgType::Telemetry {
            None
        } else {
            Some(self.bridge.telemetry_period_ms().max(1) as u64)
        }
    }

    fn has_outgoing_backlog(&self) -> bool {
        false
    }
}
