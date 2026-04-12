#[cfg(feature = "mock-bus")]
pub mod transport_mock;
#[cfg(feature = "pio-real")]
pub mod transport_pio;

use rp2040_fred_protocol::bridge_proto::Packet;

pub trait Transport {
    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize;
    fn poll_outgoing_packet(&mut self) -> Option<Packet>;
    fn post_send_delay_ms(&self, pkt: &Packet) -> Option<u64>;
    fn has_outgoing_backlog(&self) -> bool;
}