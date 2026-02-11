#![no_std]
#![no_main]

mod protocol;
mod pins;
mod transport;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

#[cfg(feature = "mock-bus")]
use crate::protocol::DroProtocolEngine;
use crate::transport::Rp2040FredTransport;

use panic_halt as _;

#[cfg(feature = "usb-bridge")]
compile_error!(
    "Feature `usb-bridge` requires aligned Embassy versions. Current workspace has embassy-rp 0.2 and embassy-usb 0.5.1, which are incompatible (different embassy-usb-driver major versions)."
);

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut transport = Rp2040FredTransport::new();
    transport.init(p);

    #[cfg(feature = "mock-bus")]
    let mut engine = DroProtocolEngine::new();

    const CADENCE: [u8; 10] = [0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0D, 0x0C];
    let mut idx = 0usize;

    loop {
        let cmd = CADENCE[idx];
        idx = (idx + 1) % CADENCE.len();

        transport.write_fc80(cmd);

        #[cfg(feature = "mock-bus")]
        {
            engine.step_telemetry();
            let reply = engine.on_command(cmd);
            transport.inject_mock_reply(reply);
        }

        let _status = transport.read_fcf0();
        let _response = transport.read_fcf1();

        Timer::after(Duration::from_micros(200)).await;
    }
}
