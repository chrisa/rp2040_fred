#[cfg(feature = "mock-bus")]
#[path = "transport_mock.rs"]
mod transport_impl;
#[cfg(feature = "pio-real")]
#[path = "transport_pio.rs"]
mod transport_impl;

#[cfg(feature = "mock-bus")]
pub use transport_impl::BridgeTransport;
#[cfg(feature = "pio-real")]
pub use transport_impl::BridgeTransport;
