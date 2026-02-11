#![no_std]
#![no_main]

#[cfg(not(feature = "usb-bridge"))]
mod protocol;
#[cfg(not(feature = "usb-bridge"))]
mod pins;
#[cfg(not(feature = "usb-bridge"))]
mod transport;

use embassy_executor::Spawner;
#[cfg(feature = "defmt-log")]
use defmt_rtt as _;
#[cfg(feature = "defmt-log")]
use panic_probe as _;
#[cfg(not(feature = "defmt-log"))]
use panic_halt as _;

#[cfg(not(feature = "usb-bridge"))]
use embassy_time::{Duration, Timer};
#[cfg(not(feature = "usb-bridge"))]
use crate::transport::Rp2040FredTransport;
#[cfg(all(not(feature = "usb-bridge"), feature = "mock-bus"))]
use crate::protocol::DroProtocolEngine;

#[cfg(feature = "usb-bridge")]
use embassy_futures::join::join;
#[cfg(feature = "usb-bridge")]
use embassy_futures::select::{select, Either};
#[cfg(feature = "usb-bridge")]
use embassy_rp::{bind_interrupts, usb};
#[cfg(feature = "usb-bridge")]
use embassy_time::{Duration, Timer};
#[cfg(feature = "usb-bridge")]
use embassy_usb::class::cmsis_dap_v2::{CmsisDapV2Class, State as CmsisState};
#[cfg(feature = "usb-bridge")]
use embassy_usb::msos;
#[cfg(feature = "usb-bridge")]
use embassy_usb::{Builder, Config};
#[cfg(feature = "usb-bridge")]
use rp2040_fred_firmware::bridge_proto::{Packet, PACKET_SIZE};
#[cfg(feature = "usb-bridge")]
use rp2040_fred_firmware::bridge_service::BridgeService;

#[cfg(feature = "defmt-log")]
macro_rules! log_info {
    ($($arg:tt)*) => {
        defmt::info!($($arg)*);
    };
}

#[cfg(not(feature = "defmt-log"))]
macro_rules! log_info {
    ($($arg:tt)*) => {};
}

#[cfg(feature = "defmt-log")]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        defmt::warn!($($arg)*);
    };
}

#[cfg(not(feature = "defmt-log"))]
macro_rules! log_warn {
    ($($arg:tt)*) => {};
}

#[cfg(feature = "usb-bridge")]
bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

#[cfg(not(feature = "usb-bridge"))]
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

#[cfg(feature = "usb-bridge")]
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    log_info!("fw start: usb-bridge");

    let driver = usb::Driver::new(p.USB, Irqs);
    log_info!("usb driver initialized");

    let mut usb_config = Config::new(0x2E8A, 0x000A);
    usb_config.manufacturer = Some("TCL125");
    usb_config.product = Some("RP2040 FRED Bridge");
    usb_config.serial_number = Some("TCL125-USB-01");
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;

    static mut CONFIG_DESCRIPTOR: [u8; 256] = [0; 256];
    static mut BOS_DESCRIPTOR: [u8; 256] = [0; 256];
    static mut MSOS_DESCRIPTOR: [u8; 256] = [0; 256];
    static mut CONTROL_BUF: [u8; 128] = [0; 128];
    static mut CMSIS_STATE: CmsisState = CmsisState::new();

    let mut builder = Builder::new(
        driver,
        usb_config,
        unsafe { &mut CONFIG_DESCRIPTOR },
        unsafe { &mut BOS_DESCRIPTOR },
        unsafe { &mut MSOS_DESCRIPTOR },
        unsafe { &mut CONTROL_BUF },
    );
    builder.msos_descriptor(msos::windows_version::WIN10, 0x20);

    let mut bridge_class = CmsisDapV2Class::new(&mut builder, unsafe { &mut CMSIS_STATE }, 64, false);
    let mut usb_device = builder.build();
    log_info!("usb descriptors built");

    let mut bridge = BridgeService::new();

    let usb_fut = usb_device.run();
    let bridge_fut = async {
        let mut rx_buf = [0u8; 64];
        let mut replies = [Packet::ping(0), Packet::ping(0)];

        loop {
            log_info!("waiting for USB host connection");
            bridge_class.wait_connection().await;
            log_info!("USB host connected");

            loop {
                match select(
                    bridge_class.read_packet(&mut rx_buf),
                    Timer::after(Duration::from_millis(2)),
                )
                .await
                {
                    Either::First(Ok(n)) => {
                        if n >= PACKET_SIZE {
                            let mut raw = [0u8; PACKET_SIZE];
                            raw.copy_from_slice(&rx_buf[..PACKET_SIZE]);

                            let reply_count = match Packet::decode(&raw) {
                                Ok(req) => bridge.handle_request(req, &mut replies),
                                Err(_) => {
                                    replies[0] = Packet::nack(0, 0xFF, 0x02);
                                    1
                                }
                            };

                            for pkt in replies.iter().take(reply_count) {
                                let encoded = pkt.encode();
                                if bridge_class.write_packet(&encoded).await.is_err() {
                                    log_warn!("USB write failed; dropping connection");
                                    break;
                                }
                            }
                        }
                    }
                    Either::First(Err(_)) => {
                        log_warn!("USB read failed; dropping connection");
                        break;
                    }
                    Either::Second(()) => {}
                }

                if let Some(pkt) = bridge.poll_telemetry_event() {
                    let encoded = pkt.encode();
                    if bridge_class.write_packet(&encoded).await.is_err() {
                        log_warn!("USB telemetry write failed; dropping connection");
                        break;
                    }
                    let period_ms = bridge.telemetry_period_ms().max(1) as u64;
                    Timer::after(Duration::from_millis(period_ms)).await;
                }
            }
        }
    };

    join(usb_fut, bridge_fut).await;
}
