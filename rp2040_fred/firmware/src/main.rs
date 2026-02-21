#![no_std]
#![no_main]

#[macro_use]
mod resources;
mod transport;

use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_rp::{bind_interrupts, usb};
use embassy_time::{Duration, Timer};
use embassy_usb::class::cmsis_dap_v2::{CmsisDapV2Class, State as CmsisState};
use embassy_usb::msos;
use embassy_usb::{Builder, Config};
use panic_probe as _;
use rp2040_fred_protocol::bridge_proto::{Packet, PACKET_SIZE};
use static_cell::StaticCell;

use crate::resources::{AssignedResources, SnifferResources, UsbResources};
use crate::transport::BridgeTransport;

#[cfg(not(feature = "defmt-log"))]
compile_error!("defmt-log feature must be enabled");

#[cfg(all(feature = "mock-bus", feature = "pio-real"))]
compile_error!("Use either `mock-bus` or `pio-real`, not both.");

macro_rules! log_info {
    ($($arg:tt)*) => {
        defmt::info!($($arg)*);
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        defmt::warn!($($arg)*);
    };
}

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let r = split_resources!(p);
    log_info!("fw start: passive-capture default");

    let mut bridge = BridgeTransport::new(r.sniffer);

    let driver = usb::Driver::new(r.usb.usb, Irqs);
    log_info!("usb driver initialized");

    let mut usb_config = Config::new(0x2E8A, 0x000A);
    usb_config.manufacturer = Some("TCL125");
    usb_config.product = Some("RP2040 FRED Bridge");
    usb_config.serial_number = Some("TCL125-USB-01");
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    static CMSIS_STATE: StaticCell<CmsisState> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        usb_config,
        CONFIG_DESCRIPTOR.init([0; 256]),
        BOS_DESCRIPTOR.init([0; 256]),
        MSOS_DESCRIPTOR.init([0; 256]),
        CONTROL_BUF.init([0; 128]),
    );
    builder.msos_descriptor(msos::windows_version::WIN10, 0x20);

    let mut bridge_class =
        CmsisDapV2Class::new(&mut builder, CMSIS_STATE.init(CmsisState::new()), 64, false);
    let mut usb_device = builder.build();
    log_info!("usb descriptors built");

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

                if let Some(pkt) = bridge.poll_outgoing_packet() {
                    let encoded = pkt.encode();
                    if bridge_class.write_packet(&encoded).await.is_err() {
                        log_warn!("USB telemetry write failed; dropping connection");
                        break;
                    }
                    if let Some(period_ms) = bridge.post_send_delay_ms(&pkt) {
                        Timer::after(Duration::from_millis(period_ms)).await;
                    }
                }
            }
        }
    };

    join(usb_fut, bridge_fut).await;
}
