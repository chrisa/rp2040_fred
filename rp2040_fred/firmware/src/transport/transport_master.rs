use core::ptr::addr_of_mut;

use embassy_executor::Executor;
use embassy_rp::multicore::Stack;
use embassy_time::{Duration, Timer};
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use static_cell::StaticCell;

use crate::resources::{Core1Resources, PioResources};
use rp2040_fred_protocol::trace_decode::{AxisSnapshot, FeedbackDecoder, FeedbackSnapshot};

mod bus;
use bus::Bus;

const FLAG_ENABLED: u8 = 1 << 0;

const TRACE_SAMPLE_RING_LEN: usize = 16_384;
const CORE1_STACK_SIZE: usize = 4096;

static TRACE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(true);
static TRACE_QUEUE_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_RXSTALL_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_SAMPLE_RING: StaticCell<Queue<u32, TRACE_SAMPLE_RING_LEN>> = StaticCell::new();
static mut CORE1_STACK: Stack<CORE1_STACK_SIZE> = Stack::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

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
    // bus: ThisBus<'a>,
}

use crate::transport::Transport;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet, TRACE_SAMPLES_PER_PACKET};

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

impl PioTransport {
    pub fn new(core1_resources: Core1Resources, pio_resources: PioResources) -> Self {
        let trace_ring = TRACE_SAMPLE_RING.init(Queue::new());
        let (producer, consumer) = trace_ring.split();

        // TRACE_CAPTURE_ENABLED.store(true, Ordering::Relaxed);
        // TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        // TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);

        embassy_rp::multicore::spawn_core1(
            core1_resources.core1,
            unsafe { &mut *addr_of_mut!(CORE1_STACK) },
            move || {
                let executor1 = EXECUTOR1.init(Executor::new());
                executor1.run(|spawner| spawner.spawn(core1_loop(pio_resources, producer).unwrap()))
            },
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

#[embassy_executor::task]
async fn core1_loop(pio_resources: PioResources, mut _trace_samples: Producer<'static, u32>) -> ! {
    let mut bus = Bus::setup(pio_resources);

    loop {
        // write/read loop:
        // command sequence: `03,02,01,00,07,06,05,04,0D,0C`
        // command is:
        // 1. Poll `F0` until bit 0 clears.
        // 2. Write one command byte to `80`.
        // 3. Poll `F0` again until bit 0 clears.
        // 4. Read one response byte from `F1`.

        for _ in 1..10 {
            bus.read_cycle(0xFF).await;
        }

        for _ in 1..10 {
            bus.write_cycle(0xFF, 0xFF).await;
        }

        for _ in 1..10 {
            bus.read_cycle(0xFF).await;
        }

        bus.read_cycle(0xFF).await;

        Timer::after(Duration::from_micros(1000)).await;
    }
}
