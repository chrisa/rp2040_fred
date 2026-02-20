#![no_std]
#![no_main]

#[cfg(feature = "pio-real")]
mod pins;
#[cfg(feature = "pio-real")]
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
#[cfg(feature = "pio-real")]
use rp2040_fred_protocol::bridge_proto::MsgType;
use rp2040_fred_protocol::bridge_proto::{Packet, PACKET_SIZE};
#[cfg(feature = "mock-bus")]
use rp2040_fred_protocol::bridge_service::BridgeService;
#[cfg(feature = "pio-real")]
use rp2040_fred_protocol::dro_decode::{DroAssembler, DroSnapshot};
use static_cell::StaticCell;
#[cfg(feature = "pio-real")]
use transport::Rp2040FredTransport;

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

#[cfg(feature = "pio-real")]
const DRO_CADENCE: [u8; 10] = [0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0D, 0x0C];

#[cfg(feature = "pio-real")]
const FLAG_ENABLED: u8 = 1 << 0;

#[cfg(feature = "pio-real")]
struct LiveBridge {
    telemetry_enabled: bool,
    telemetry_period_ms: u16,
    tick: u32,
    telemetry_seq: u16,
    cadence_idx: usize,
    dro: DroAssembler,
    last_snapshot: DroSnapshot,
}

#[cfg(feature = "pio-real")]
impl LiveBridge {
    const fn new() -> Self {
        Self {
            telemetry_enabled: false,
            telemetry_period_ms: 100,
            tick: 0,
            telemetry_seq: 1,
            cadence_idx: 0,
            dro: DroAssembler::new(),
            last_snapshot: DroSnapshot {
                x_counts: 0,
                z_counts: 0,
                rpm: 0,
            },
        }
    }

    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            MsgType::TelemetrySet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::TelemetrySet as u8, 1);
                    return 1;
                }
                self.telemetry_enabled = req.payload[0] != 0;
                if req.payload_len >= 3 {
                    self.telemetry_period_ms = u16::from_le_bytes([req.payload[1], req.payload[2]]);
                }
                out[0] = Packet::ack(req.seq, MsgType::TelemetrySet, 0);
                1
            }
            MsgType::SnapshotReq => {
                let s = self.last_snapshot;
                out[0] = Packet::telemetry(
                    req.seq,
                    self.tick,
                    s.x_counts,
                    s.z_counts,
                    s.rpm,
                    self.flags(),
                );
                out[1] = Packet::ack(req.seq, MsgType::SnapshotReq, 0);
                2
            }
            _ => {
                out[0] = Packet::nack(req.seq, req.msg_type as u8, 0xFE);
                1
            }
        }
    }

    fn poll_telemetry_event(&mut self, transport: &mut Rp2040FredTransport) -> Option<Packet> {
        if !self.telemetry_enabled {
            return None;
        }

        let cmd = DRO_CADENCE[self.cadence_idx];
        self.cadence_idx = (self.cadence_idx + 1) % DRO_CADENCE.len();
        transport.write_fc80(cmd);
        let _status = transport.read_fcf0();
        let response = transport.read_fcf1();

        self.tick = self.tick.wrapping_add(1);
        self.dro.on_fc80_fcf1(cmd, response);
        if cmd != 0x0C {
            return None;
        }

        self.last_snapshot = self.dro.snapshot();
        let s = self.last_snapshot;
        let pkt = Packet::telemetry(
            self.telemetry_seq,
            self.tick,
            s.x_counts,
            s.z_counts,
            s.rpm,
            self.flags(),
        );
        self.telemetry_seq = self.telemetry_seq.wrapping_add(1);
        Some(pkt)
    }

    fn telemetry_period_ms(&self) -> u16 {
        self.telemetry_period_ms
    }

    fn flags(&self) -> u8 {
        if self.telemetry_enabled {
            FLAG_ENABLED
        } else {
            0
        }
    }
}

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<embassy_rp::peripherals::USB>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    log_info!("fw start: usb-bridge");

    #[cfg(feature = "pio-real")]
    let mut fred_transport = {
        let mut transport = Rp2040FredTransport::new();
        transport.init_pio_peripherals(
            p.PIO0, p.PIN_0, p.PIN_1, p.PIN_2, p.PIN_3, p.PIN_4, p.PIN_5, p.PIN_6, p.PIN_7,
            p.PIN_8, p.PIN_9, p.PIN_10, p.PIN_11, p.PIN_12, p.PIN_13, p.PIN_14, p.PIN_15, p.PIN_16,
            p.PIN_17, p.PIN_20, p.PIN_27, p.PIN_28,
        );
        transport
    };

    let driver = usb::Driver::new(p.USB, Irqs);
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

    #[cfg(feature = "mock-bus")]
    let mut bridge = BridgeService::new();
    #[cfg(feature = "pio-real")]
    let mut bridge = LiveBridge::new();

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

                #[cfg(feature = "mock-bus")]
                let telemetry_pkt = bridge.poll_telemetry_event();
                #[cfg(feature = "pio-real")]
                let telemetry_pkt = bridge.poll_telemetry_event(&mut fred_transport);

                if let Some(pkt) = telemetry_pkt {
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
