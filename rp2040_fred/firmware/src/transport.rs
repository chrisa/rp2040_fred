pub mod master;
pub mod passive;

mod pio;

use rp2040_fred_protocol::bridge_proto::Packet;

use crate::transport::{master::BusMasterTransport, passive::PassiveTransport};

pub enum Transport {
    Passive(PassiveTransport),
    Master(BusMasterTransport),
}

impl Transport {
    pub fn handle_master_request(&mut self, req: &Packet, out: &mut [Packet; 2]) -> usize {
        match self {
            Self::Passive(t) => t.handle_master_request(req, out),
            Self::Master(t) => t.handle_master_request(req, out),
        }
    }

    pub fn handle_capture_request(&mut self, req: &Packet, out: &mut [Packet; 2]) -> usize {
        match self {
            Self::Passive(t) => t.handle_capture_request(req, out),
            Self::Master(t) => t.handle_capture_request(req, out),
        }
    }

    pub fn process_master_pending_work(&mut self, budget: usize) {
        match self {
            Self::Passive(t) => t.process_master_pending_work(budget),
            Self::Master(t) => t.process_master_pending_work(budget),
        }
    }

    pub fn poll_master_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet> {
        match self {
            Self::Passive(t) => t.poll_master_outgoing_packet(now_ms),
            Self::Master(t) => t.poll_master_outgoing_packet(now_ms),
        }
    }

    pub fn poll_capture_outgoing_packet(&mut self) -> Option<Packet> {
        match self {
            Self::Passive(t) => t.poll_capture_outgoing_packet(),
            Self::Master(t) => t.poll_capture_outgoing_packet(),
        }
    }

    pub fn has_master_decode_work(&self) -> bool {
        match self {
            Self::Passive(t) => t.has_master_decode_work(),
            Self::Master(t) => t.has_master_decode_work(),
        }
    }

    pub fn has_master_outgoing_packet(&self, now_ms: u64) -> bool {
        match self {
            Self::Passive(t) => t.has_master_outgoing_packet(now_ms),
            Self::Master(t) => t.has_master_outgoing_packet(now_ms),
        }
    }

    pub fn has_capture_outgoing_packet(&self) -> bool {
        match self {
            Self::Passive(t) => t.has_capture_outgoing_packet(),
            Self::Master(t) => t.has_capture_outgoing_packet(),
        }
    }
}
