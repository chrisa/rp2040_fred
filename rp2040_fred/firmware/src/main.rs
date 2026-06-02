#![no_std]
#![no_main]

#[macro_use]
mod resources;

mod decoder;
mod transport;
mod usb_bulk;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::{select3, Either3};
use embassy_rp::peripherals::{PIO0, PIO1};
use embassy_rp::pio::InterruptHandler;
use embassy_rp::{bind_interrupts, usb};
use embassy_rp::{clocks::ClockConfig, gpio};
use embassy_time::{Duration, Instant, Timer};
use embassy_usb::msos;
use embassy_usb::{Builder, Config as UsbConfig};

use gpio::{Level, Output};
use panic_probe as _;
use rp2040_fred_firmware::{log_info, log_warn};
use rp2040_fred_protocol::bridge_proto::{
    Packet, MIN_PACKET_SIZE, PACKET_SIZE, USB_PROTOCOL_CAPTURE, USB_PROTOCOL_MASTER,
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use crate::resources::{
    AssignedResources, Core1Resources, DebugPin27Resources, DebugPin28Resources,
    DirectionResources, MainResources, PioResources, UsbResources,
};
use crate::transport::{master::BusMasterTransport, passive::PassiveTransport, Transport};
use crate::usb_bulk::{VendorBulkInterface, VendorBulkState};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

const USB_IDLE_POLL_MS: u64 = 10;
const USB_BACKLOG_POLL_US: u64 = 50;
const USB_MASTER_OUTGOING_BURST_PACKETS: usize = 16;
const USB_CAPTURE_OUTGOING_BURST_PACKETS: usize = 4;
const USB_DECODE_BURST_SAMPLES: usize = 512;

#[expect(dead_code, reason = "startup mode selection is edited during bring-up")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportMode {
    Passive,
    Master,
}

const TRANSPORT_MODE: TransportMode = TransportMode::Master;

defmt::timestamp!("{=u64:us}", {
    // NOTE(interrupt-safe) single instruction volatile read operation
    Instant::now().as_micros()
});

bind_interrupts!(pub struct PioIrqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let clock_config = ClockConfig::system_freq(125_000_000).expect("set clock failed?");
    let config = embassy_rp::config::Config::new(clock_config);
    let p = embassy_rp::init(config);
    let r = split_resources!(p);

    let mut led = Output::new(r.main.led, Level::Low);
    led.set_high();

    let driver = usb::Driver::new(r.usb.usb, Irqs);
    log_info!("usb driver initialized");

    let mut transport = match TRANSPORT_MODE {
        TransportMode::Passive => {
            Transport::Passive(PassiveTransport::new(r.core1, r.pio, r.dir, r.debug27))
        }
        TransportMode::Master => Transport::Master(BusMasterTransport::new(
            r.core1, r.pio, r.dir, r.debug27, r.debug28,
        )),
    };

    let mut usb_config = UsbConfig::new(0x2E8A, 0x000A);
    usb_config.manufacturer = Some("TCL125");
    usb_config.product = Some("RP2040 FRED Bridge");
    usb_config.serial_number = Some("TCL125-USB-01");
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESCRIPTOR: StaticCell<[u8; 512]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    static MASTER_USB_STATE: StaticCell<VendorBulkState> = StaticCell::new();
    static CAPTURE_USB_STATE: StaticCell<VendorBulkState> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        usb_config,
        CONFIG_DESCRIPTOR.init([0; 256]),
        BOS_DESCRIPTOR.init([0; 256]),
        MSOS_DESCRIPTOR.init([0; 512]),
        CONTROL_BUF.init([0; 128]),
    );
    builder.msos_descriptor(msos::windows_version::WIN10, 0x20);

    let mut master_usb = VendorBulkInterface::new(
        &mut builder,
        MASTER_USB_STATE.init(VendorBulkState::new()),
        USB_PROTOCOL_MASTER,
        "TCL125 Master",
        "{8BDB8E41-8F7D-4F9B-9E6D-4C2AA4C75D5A}",
        64,
    );
    let mut capture_usb = VendorBulkInterface::new(
        &mut builder,
        CAPTURE_USB_STATE.init(VendorBulkState::new()),
        USB_PROTOCOL_CAPTURE,
        "TCL125 Capture",
        "{C34A0F38-0F85-4CF8-8EA9-09F40C7A7A46}",
        64,
    );
    let mut usb_device = builder.build();
    log_info!("usb descriptors built");

    let usb_fut = usb_device.run();
    let bridge_fut = async {
        let mut master_rx_buf = [0_u8; PACKET_SIZE];
        let mut capture_rx_buf = [0_u8; PACKET_SIZE];
        let mut master_replies = [Packet::ping(0), Packet::ping(0)];
        let mut capture_replies = [Packet::ping(0), Packet::ping(0)];

        loop {
            log_info!("waiting for USB host connection");
            master_usb.wait_connection().await;
            capture_usb.wait_connection().await;
            log_info!("USB host connected");

            'connected: loop {
                let now_ms = Instant::now().as_millis();
                match select3(
                    master_usb.read_packet(&mut master_rx_buf),
                    capture_usb.read_packet(&mut capture_rx_buf),
                    if transport.has_master_decode_work()
                        || transport.has_master_outgoing_packet(now_ms)
                        || transport.has_capture_outgoing_packet()
                    {
                        Timer::after(Duration::from_micros(USB_BACKLOG_POLL_US))
                    } else {
                        Timer::after(Duration::from_millis(USB_IDLE_POLL_MS))
                    },
                )
                .await
                {
                    Either3::First(Ok(n)) => {
                        if n >= MIN_PACKET_SIZE {
                            let reply_count = if let Ok(req) = Packet::decode(&master_rx_buf[..n]) {
                                transport.handle_master_request(&req, &mut master_replies)
                            } else {
                                master_replies[0] = Packet::nack(0, 0xFF, 0x02);
                                1
                            };

                            for pkt in master_replies.iter().take(reply_count) {
                                let encoded = pkt.encode();
                                let encoded_len = pkt.encoded_len();
                                if master_usb
                                    .write_packet(&encoded[..encoded_len])
                                    .await
                                    .is_err()
                                {
                                    log_warn!("master USB write failed; dropping connection");
                                    break 'connected;
                                }
                                log_info!("wrote master request/response USB packet OK");
                            }
                        }
                    }
                    Either3::First(Err(_)) => {
                        log_warn!("master USB read failed; dropping connection");
                        break;
                    }
                    Either3::Second(Ok(n)) => {
                        if n >= MIN_PACKET_SIZE {
                            let reply_count = if let Ok(req) = Packet::decode(&capture_rx_buf[..n])
                            {
                                transport.handle_capture_request(&req, &mut capture_replies)
                            } else {
                                capture_replies[0] = Packet::nack(0, 0xFF, 0x02);
                                1
                            };

                            for pkt in capture_replies.iter().take(reply_count) {
                                let encoded = pkt.encode();
                                let encoded_len = pkt.encoded_len();
                                if capture_usb
                                    .write_packet(&encoded[..encoded_len])
                                    .await
                                    .is_err()
                                {
                                    log_warn!("capture USB write failed; dropping connection");
                                    break 'connected;
                                }
                                log_info!("wrote capture request/response USB packet OK");
                            }
                        }
                    }
                    Either3::Second(Err(_)) => {
                        log_warn!("capture USB read failed; dropping connection");
                        break;
                    }
                    Either3::Third(()) => {}
                }

                transport.process_master_pending_work(USB_DECODE_BURST_SAMPLES);

                for _ in 0..USB_MASTER_OUTGOING_BURST_PACKETS {
                    let now_ms = Instant::now().as_millis();
                    let Some(pkt) = transport.poll_master_outgoing_packet(now_ms) else {
                        break;
                    };
                    let encoded = pkt.encode();
                    let encoded_len = pkt.encoded_len();
                    if master_usb
                        .write_packet(&encoded[..encoded_len])
                        .await
                        .is_err()
                    {
                        log_warn!("master USB telemetry write failed; dropping connection");
                        break 'connected;
                    }
                }

                for _ in 0..USB_CAPTURE_OUTGOING_BURST_PACKETS {
                    let Some(pkt) = transport.poll_capture_outgoing_packet() else {
                        break;
                    };
                    let encoded = pkt.encode();
                    let encoded_len = pkt.encoded_len();
                    if capture_usb
                        .write_packet(&encoded[..encoded_len])
                        .await
                        .is_err()
                    {
                        log_warn!("capture USB trace write failed; dropping connection");
                        break 'connected;
                    }
                }
            }
        }
    };

    join(usb_fut, bridge_fut).await;
}
