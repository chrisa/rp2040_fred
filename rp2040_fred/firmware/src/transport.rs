pub mod master;
pub mod passive;

mod pio;

use enum_dispatch::enum_dispatch;
use rp2040_fred_protocol::bridge_proto::Packet;

use crate::transport::{master::BusMasterTransport, passive::PassiveTransport};

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
    Passive(PassiveTransport),
    Master(BusMasterTransport),
}
