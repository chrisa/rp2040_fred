use crate::transport::Transport;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};

mod bridge_service;
mod mock_bus;
mod protocol;

use bridge_service::BridgeService;

pub struct MockTransport {
    bridge: BridgeService,
    next_due_ms: u64,
}

impl MockTransport {
    pub fn new() -> Self {
        Self {
            bridge: BridgeService::new(),
            next_due_ms: 0,
        }
    }
}

impl Transport for MockTransport {
    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
        self.next_due_ms = 0;
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            _ => self.bridge.handle_request(req, out),
        }
    }

    fn process_pending_work(&mut self, _budget: usize) {}

    fn poll_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet> {
        if now_ms < self.next_due_ms {
            return None;
        }

        let pkt = self.bridge.poll_outgoing_packet()?;
        if pkt.msg_type == MsgType::Telemetry {
            self.next_due_ms = now_ms + self.bridge.telemetry_period_ms().max(1) as u64;
        } else {
            self.next_due_ms = now_ms;
        }
        Some(pkt)
    }

    fn has_decode_work(&self) -> bool {
        false
    }

    fn has_outgoing_packet(&self, now_ms: u64) -> bool {
        now_ms >= self.next_due_ms
    }
}
