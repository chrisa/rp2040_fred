#[cfg(feature = "mock-bus")]
pub mod transport_mock;
#[cfg(feature = "pio-real")]
pub mod transport_pio;

use rp2040_fred_protocol::bridge_proto::Packet;

pub trait Transport {
    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize;
    fn process_pending_work(&mut self, budget: usize);
    fn poll_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet>;
    fn has_decode_work(&self) -> bool;
    fn has_outgoing_packet(&self, now_ms: u64) -> bool;
}
