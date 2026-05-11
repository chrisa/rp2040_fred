pub mod master;
pub mod mock;
pub mod passive;

use enum_dispatch::enum_dispatch;
use rp2040_fred_protocol::bridge_proto::Packet;

use crate::transport::{
    master::BusMasterTransport, mock::MockTransport, passive::PassiveTransport,
};

#[enum_dispatch]
pub trait GenericTransport {
    fn handle_request(&mut self, req: &Packet, out: &mut [Packet; 2]) -> usize;
    fn process_pending_work(&mut self, budget: usize);
    fn poll_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet>;
    fn has_decode_work(&self) -> bool;
    fn has_outgoing_packet(&self, now_ms: u64) -> bool;
}

#[enum_dispatch(GenericTransport)]
pub enum Transport {
    Mock(MockTransport),
    Passive(PassiveTransport),
    Master(BusMasterTransport),
}
