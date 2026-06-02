use core::ptr::addr_of_mut;

use embassy_executor::Executor;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::multicore::Stack;
use embassy_time::{Duration, Instant, Timer};
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use rp2040_fred_firmware::{log_info, log_warn};
use static_cell::StaticCell;

use crate::decoder::{FeedbackDecoder, FeedbackSnapshot};
use crate::resources::{Core1Resources, DebugPin27Resources, DirectionResources, PioResources};
use crate::transport::pio::passive::PassivePio;

use rp2040_fred_protocol::bridge_proto::{MsgType, Packet, TRACE_SAMPLES_PER_PACKET};

const FLAG_ENABLED: u8 = 1 << 0;

const TRACE_SAMPLE_RING_LEN: usize = 16_384;
const CORE1_STACK_SIZE: usize = 4096;

static TRACE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);
static TRACE_QUEUE_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_RXSTALL_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_SAMPLE_RING: StaticCell<Queue<u32, TRACE_SAMPLE_RING_LEN>> = StaticCell::new();
static mut CORE1_STACK: Stack<CORE1_STACK_SIZE> = Stack::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

pub struct PassiveTransport {
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

impl PassiveTransport {
    pub fn new(
        core1_resources: Core1Resources,
        pio_resources: PioResources,
        dir_resources: DirectionResources,
        debug_resources: DebugPin27Resources,
    ) -> Self {
        let trace_ring = TRACE_SAMPLE_RING.init(Queue::new());
        let (producer, consumer) = trace_ring.split();

        TRACE_CAPTURE_ENABLED.store(false, Ordering::Relaxed);
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);

        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "standard pattern, can't move out of CORE1_STACK"
        )]
        // SAFETY: standard core1 stack init pattern
        let stack = unsafe { &mut *addr_of_mut!(CORE1_STACK) };

        embassy_rp::multicore::spawn_core1(core1_resources.core1, stack, move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| {
                spawner.spawn(
                    capture_core1_loop(pio_resources, dir_resources, debug_resources, producer)
                        .expect("spawn capture_core1_loop"),
                );
            })
        });

        Self {
            trace_samples: consumer,
            capture_enabled: false,
            telemetry_enabled: false,
            packet_seq: 1,
            sample_seq: 0,
            decoder: FeedbackDecoder::new(),
            current_snapshot: FeedbackSnapshot::default(),
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
        self.current_snapshot = FeedbackSnapshot::default();
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

impl PassiveTransport {
    pub fn handle_master_request(&mut self, req: &Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            MsgType::TelemetrySet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::TelemetrySet as u8, 1);
                } else {
                    self.telemetry_enabled = req.payload[0] != 0;
                    if self.telemetry_enabled {
                        self.capture_enabled = false;
                    }
                    TRACE_CAPTURE_ENABLED.store(
                        self.telemetry_enabled || self.capture_enabled,
                        Ordering::Relaxed,
                    );
                    self.reset_stream_state();
                    if req.payload_len >= 3 {
                        self.telemetry_period_ms =
                            u16::from_le_bytes([req.payload[1], req.payload[2]]);
                    }
                    out[0] = Packet::ack(req.seq, MsgType::TelemetrySet, 0);
                    log_info!(
                        "telemetry_enabled: {} capture_enabled: {}",
                        self.telemetry_enabled,
                        self.capture_enabled
                    );
                }
                1
            }
            _ => {
                out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x21);
                log_warn!("nacked passive master request 0x{:x}", req.msg_type as u8);
                1
            }
        }
    }

    pub fn handle_capture_request(&mut self, req: &Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            MsgType::CaptureSet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::CaptureSet as u8, 1);
                } else {
                    self.capture_enabled = req.payload[0] != 0;
                    if self.capture_enabled {
                        self.telemetry_enabled = false;
                    }
                    TRACE_CAPTURE_ENABLED.store(
                        self.telemetry_enabled || self.capture_enabled,
                        Ordering::Relaxed,
                    );
                    self.reset_stream_state();
                    out[0] = Packet::ack(req.seq, MsgType::CaptureSet, 0);
                    log_info!(
                        "telemetry_enabled: {} capture_enabled: {}",
                        self.telemetry_enabled,
                        self.capture_enabled
                    );
                }
                1
            }
            _ => {
                out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x22);
                log_warn!("nacked passive capture request 0x{:x}", req.msg_type as u8);
                1
            }
        }
    }

    pub fn process_master_pending_work(&mut self, budget: usize) {
        if !self.telemetry_enabled {
            return;
        }

        let mut processed = 0_usize;
        while processed < budget {
            let Some(sample) = self.trace_samples.dequeue() else {
                break;
            };

            match self.decoder.ingest_sample(self.sample_seq, sample) {
                Ok(s) => {
                    self.current_snapshot = s;
                    self.snapshot_valid = true;
                }
                Err(_e) => {
                    // Keep the last good snapshot.
                }
            }
            self.sample_seq = self.sample_seq.wrapping_add(1);
            processed += 1;
        }
    }

    pub fn poll_master_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet> {
        if !self.telemetry_enabled {
            return None;
        }

        if !self.snapshot_valid || now_ms < self.next_telemetry_due_ms {
            return None;
        }

        let pkt = Packet::telemetry(
            self.packet_seq,
            0,
            self.current_snapshot.x.count(),
            self.current_snapshot.z.count(),
            self.current_snapshot.s.rpm(),
            self.flags(),
        );
        self.packet_seq = self.packet_seq.wrapping_add(1);
        self.next_telemetry_due_ms = now_ms + u64::from(self.telemetry_period_ms.max(1));
        Some(pkt)
    }

    pub fn poll_capture_outgoing_packet(&mut self) -> Option<Packet> {
        if !self.capture_enabled {
            return None;
        }

        let mut batch = [0_u32; TRACE_SAMPLES_PER_PACKET];
        let mut used = 0_usize;

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
        let timestamp_us = Instant::now().as_micros();
        let pkt = Packet::trace_samples(
            self.packet_seq,
            Some(timestamp_us),
            dropped_samples_total,
            rx_stall_count_total,
            &batch[..used],
        );
        self.packet_seq = self.packet_seq.wrapping_add(1);
        Some(pkt)
    }

    pub fn has_master_decode_work(&self) -> bool {
        self.telemetry_enabled && self.trace_samples.ready()
    }

    pub fn has_master_outgoing_packet(&self, now_ms: u64) -> bool {
        self.telemetry_enabled && self.snapshot_valid && now_ms >= self.next_telemetry_due_ms
    }

    pub fn has_capture_outgoing_packet(&self) -> bool {
        self.capture_enabled && self.trace_samples.ready()
    }
}

#[embassy_executor::task]
async fn capture_core1_loop(
    pio_resources: PioResources,
    dir_resources: DirectionResources,
    debug_resources: DebugPin27Resources,
    mut trace_samples: Producer<'static, u32>,
) -> ! {
    let mut data_dir_d = Output::new(dir_resources.pin_19, Level::Low);
    let mut data_dir_a = Output::new(dir_resources.pin_20, Level::Low);
    let mut data_dir_c = Output::new(dir_resources.pin_21, Level::Low);
    data_dir_d.set_low();
    data_dir_a.set_low();
    data_dir_c.set_low();

    let mut pio = PassivePio::setup(pio_resources, debug_resources.pin);

    loop {
        if !TRACE_CAPTURE_ENABLED.load(Ordering::Relaxed) {
            Timer::after(Duration::from_micros(100)).await;
            continue;
        }

        let sample = pio.read.rx().wait_pull().await;

        if trace_samples.enqueue(sample).is_err() {
            TRACE_QUEUE_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        if pio.read.rx().stalled() {
            TRACE_RXSTALL_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }
}
