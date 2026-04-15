use core::hint::spin_loop;
use core::ptr::addr_of_mut;

use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{
    Config, Direction, InterruptHandler, Pio, PioBatch, ShiftConfig, ShiftDirection,
};
use embassy_rp::pio_programs::clock_divider::calculate_pio_clock_divider_value;
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use static_cell::StaticCell;

use crate::resources::{Core1Resources, SnifferResources};
use crate::transport::Transport;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet, TRACE_SAMPLES_PER_PACKET};
use rp2040_fred_protocol::trace_decode::{AxisSnapshot, FeedbackDecoder, FeedbackSnapshot};

macro_rules! log_info {
    ($($arg:tt)*) => {
        defmt::info!($($arg)*);
    };
}

bind_interrupts!(struct PioIrqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

const FLAG_ENABLED: u8 = 1 << 0;

const TRACE_SAMPLE_RING_LEN: usize = 16_384;
const CORE1_STACK_SIZE: usize = 4096;

static TRACE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(true);
static TRACE_QUEUE_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_RXSTALL_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_SAMPLE_RING: StaticCell<Queue<u32, TRACE_SAMPLE_RING_LEN>> = StaticCell::new();
static mut CORE1_STACK: Stack<CORE1_STACK_SIZE> = Stack::new();

pub struct PioTransport {
    trace_samples: Consumer<'static, u32>,
    capture_enabled: bool,
    telemetry_enabled: bool,
    packet_seq: u16,
    sample_seq: u64,
    decoder: FeedbackDecoder,
    current_snapshot: FeedbackSnapshot,
    snapshot_valid: bool,
    telemetry_period_ms: u16,
    next_telemetry_due_ms: u64,
}

impl PioTransport {
    pub fn new(core1_resources: Core1Resources, sniffer_resources: SnifferResources) -> Self {
        let trace_ring = TRACE_SAMPLE_RING.init(Queue::new());
        let (producer, consumer) = trace_ring.split();

        TRACE_CAPTURE_ENABLED.store(true, Ordering::Relaxed);
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);

        spawn_core1(
            core1_resources.core1,
            unsafe { &mut *addr_of_mut!(CORE1_STACK) },
            move || capture_core1_loop(sniffer_resources, producer),
        );

        Self {
            trace_samples: consumer,
            capture_enabled: false,
            telemetry_enabled: false,
            packet_seq: 1,
            sample_seq: 0,
            decoder: FeedbackDecoder::new(),
            current_snapshot: FeedbackSnapshot {
                sample_index: 0,
                x: AxisSnapshot {
                    negative: false,
                    value: 0,
                },
                z: AxisSnapshot {
                    negative: false,
                    value: 0,
                },
                rpm_display: 0,
                rpm_raw: 0,
            },
            snapshot_valid: false,
            telemetry_period_ms: 100,
            next_telemetry_due_ms: 0,
        }
    }

    fn clear_trace_samples(&mut self) {
        while self.trace_samples.dequeue().is_some() {}
    }

    fn reset_stream_state(&mut self) {
        self.packet_seq = 1;
        self.sample_seq = 0;
        self.decoder = FeedbackDecoder::new();
        self.current_snapshot = FeedbackSnapshot {
            sample_index: 0,
            x: AxisSnapshot {
                negative: false,
                value: 0,
            },
            z: AxisSnapshot {
                negative: false,
                value: 0,
            },
            rpm_display: 0,
            rpm_raw: 0,
        };
        self.snapshot_valid = false;
        self.next_telemetry_due_ms = 0;
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);
        self.clear_trace_samples();
    }

    fn flags(&self) -> u8 {
        if self.telemetry_enabled {
            FLAG_ENABLED
        } else {
            0
        }
    }
}

impl Transport for PioTransport {
    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            MsgType::CaptureSet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::CaptureSet as u8, 1);
                } else {
                    self.telemetry_enabled = req.payload[0] == 0;
                    self.capture_enabled = req.payload[0] != 0;
                    TRACE_CAPTURE_ENABLED.store(self.capture_enabled, Ordering::Relaxed);
                    self.reset_stream_state();
                    out[0] = Packet::ack(req.seq, MsgType::CaptureSet, 0);
                }
                1
            }
            MsgType::TelemetrySet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::TelemetrySet as u8, 1);
                } else {
                    self.capture_enabled = req.payload[0] == 0;
                    self.telemetry_enabled = req.payload[0] != 0;
                    TRACE_CAPTURE_ENABLED.store(self.telemetry_enabled, Ordering::Relaxed); // weird, but must capture to decode
                    self.reset_stream_state();
                    if req.payload_len >= 3 {
                        self.telemetry_period_ms =
                            u16::from_le_bytes([req.payload[1], req.payload[2]]);
                    }
                    out[0] = Packet::ack(req.seq, MsgType::TelemetrySet, 0);
                }
                1
            }
            _ => {
                if self.capture_enabled {
                    out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x10);
                } else {
                    out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x11);
                }
                1
            }
        }
    }

    fn process_pending_work(&mut self, budget: usize) {
        if !self.telemetry_enabled {
            return;
        }

        let mut processed = 0usize;
        while processed < budget {
            let Some(sample) = self.trace_samples.dequeue() else {
                break;
            };

            if let Some(snapshot) = self.decoder.ingest_sample(self.sample_seq, sample) {
                self.current_snapshot = snapshot;
                self.snapshot_valid = true;
            }
            self.sample_seq = self.sample_seq.wrapping_add(1);
            processed += 1;
        }
    }

    fn poll_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet> {
        if self.capture_enabled {
            let mut batch = [0u32; TRACE_SAMPLES_PER_PACKET];
            let mut used = 0usize;

            while used < batch.len() {
                let Some(sample) = self.trace_samples.dequeue() else {
                    break;
                };
                batch[used] = sample;
                used += 1;
            }

            if used == 0 {
                return None;
            }

            let dropped_samples_total = TRACE_QUEUE_DROP_COUNT.load(Ordering::Relaxed);
            let rx_stall_count_total = TRACE_RXSTALL_COUNT.load(Ordering::Relaxed);
            let pkt = Packet::trace_samples(
                self.packet_seq,
                dropped_samples_total,
                rx_stall_count_total,
                &batch[..used],
            );
            self.packet_seq = self.packet_seq.wrapping_add(1);
            return Some(pkt);
        }

        if self.telemetry_enabled {
            if !self.snapshot_valid || now_ms < self.next_telemetry_due_ms {
                return None;
            }

            let pkt = Packet::telemetry(
                self.packet_seq,
                0,
                self.current_snapshot.x.count(),
                self.current_snapshot.z.count(),
                self.current_snapshot.rpm_display,
                self.flags(),
            );
            self.packet_seq = self.packet_seq.wrapping_add(1);
            self.next_telemetry_due_ms = now_ms + self.telemetry_period_ms.max(1) as u64;
            return Some(pkt);
        }

        None
    }

    fn has_decode_work(&self) -> bool {
        self.telemetry_enabled && self.trace_samples.ready()
    }

    fn has_outgoing_packet(&self, now_ms: u64) -> bool {
        if self.capture_enabled {
            return self.trace_samples.ready();
        }
        self.telemetry_enabled && self.snapshot_valid && now_ms >= self.next_telemetry_due_ms
    }
}

fn capture_core1_loop(
    sniffer_resources: SnifferResources,
    mut trace_samples: Producer<'static, u32>,
) -> ! {
    let mut data_dir_a = Output::new(sniffer_resources.pin_26, Level::Low);
    let mut data_dir_d = Output::new(sniffer_resources.pin_27, Level::Low);
    data_dir_a.set_low();
    data_dir_d.set_low();

    let program = pio::pio_file!(
        "../pio/passive_sniffer.pio",
        select_program("fred_passive_sniffer"),
        options(max_program_size = 32)
    );

    let mut pio = Pio::new(sniffer_resources.pio0, PioIrqs);

    let loaded = pio.common.load_program(&program.program);

    let p0 = pio.common.make_pio_pin(sniffer_resources.pin_0);
    let p1 = pio.common.make_pio_pin(sniffer_resources.pin_1);
    let p2 = pio.common.make_pio_pin(sniffer_resources.pin_2);
    let p3 = pio.common.make_pio_pin(sniffer_resources.pin_3);
    let p4 = pio.common.make_pio_pin(sniffer_resources.pin_4);
    let p5 = pio.common.make_pio_pin(sniffer_resources.pin_5);
    let p6 = pio.common.make_pio_pin(sniffer_resources.pin_6);
    let p7 = pio.common.make_pio_pin(sniffer_resources.pin_7);
    let p8 = pio.common.make_pio_pin(sniffer_resources.pin_8);
    let p9 = pio.common.make_pio_pin(sniffer_resources.pin_9);
    let p10 = pio.common.make_pio_pin(sniffer_resources.pin_10);
    let p11 = pio.common.make_pio_pin(sniffer_resources.pin_11);
    let p12 = pio.common.make_pio_pin(sniffer_resources.pin_12);
    let p13 = pio.common.make_pio_pin(sniffer_resources.pin_13);
    let p14 = pio.common.make_pio_pin(sniffer_resources.pin_14);
    let p15 = pio.common.make_pio_pin(sniffer_resources.pin_15);
    let p16 = pio.common.make_pio_pin(sniffer_resources.pin_16);
    let p17 = pio.common.make_pio_pin(sniffer_resources.pin_17);
    let p20 = pio.common.make_pio_pin(sniffer_resources.pin_20);
    let p28 = pio.common.make_pio_pin(sniffer_resources.pin_28);

    let in_pins = [
        &p0, &p1, &p2, &p3, &p4, &p5, &p6, &p7, // data bus
        &p8, &p9, &p10, &p11, &p12, &p13, &p14, &p15, // addr bus
        &p16, // RnW
        &p17, // 1MHz
    ];

    let mut cfg = Config::default();
    cfg.use_program(&loaded, &[&p28]);
    cfg.set_in_pins(&in_pins);
    cfg.set_jmp_pin(&p20);
    cfg.shift_in = ShiftConfig {
        threshold: 32,
        direction: ShiftDirection::Left,
        auto_fill: false,
    };
    cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 20_000_000);

    pio.sm2.set_config(&cfg);
    pio.sm2.set_pin_dirs(Direction::In, &in_pins);
    pio.sm2.set_pin_dirs(Direction::Out, &[&p28]);
    pio.sm2.clear_fifos();

    let mut batch = PioBatch::new();
    batch.restart(&mut pio.sm2);
    batch.set_enable(&mut pio.sm2, true);
    batch.execute();

    let _ = pio.sm2.rx().stalled();
    log_info!("PIO initialised on core1");

    loop {
        let mut drained = false;
        while let Some(raw_sample) = pio.sm2.rx().try_pull() {
            drained = true;

            if !TRACE_CAPTURE_ENABLED.load(Ordering::Relaxed) {
                continue;
            }

            let sample = encode_trace_sample(raw_sample);
            if trace_samples.enqueue(sample).is_err() {
                TRACE_QUEUE_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        if pio.sm2.rx().stalled() {
            TRACE_RXSTALL_COUNT.fetch_add(1, Ordering::Relaxed);
        }

        if !drained {
            spin_loop();
        }
    }
}

#[inline]
fn encode_trace_sample(raw_sample: u32) -> u32 {
    // The non-consecutive hardware map keeps bus bits on GPIO0..17 and uses
    // GPIO20 for FRED_N, leaving GPIO18/19 intentionally unused.
    raw_sample
}
