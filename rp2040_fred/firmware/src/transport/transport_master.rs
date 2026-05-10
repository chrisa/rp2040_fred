use core::ptr::addr_of_mut;

use embassy_executor::Executor;
use embassy_rp::bind_interrupts;
use embassy_rp::multicore::Stack;
use embassy_rp::peripherals::PIO1;
use embassy_rp::pio::{
    Config, Direction, InterruptHandler, Pio, PioBatch, ShiftConfig, ShiftDirection,
};
use embassy_rp::pio_programs::clock_divider::calculate_pio_clock_divider_value;
use embassy_time::{Duration, Instant, Timer};
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use rp2040_fred_firmware::{log_info, log_warn};
use static_cell::StaticCell;

use crate::resources::{Core1Resources, PioResources};
use crate::transport::Transport;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet, TRACE_SAMPLES_PER_PACKET};
use rp2040_fred_protocol::trace_decode::{
    AxisSnapshot, FeedbackCommand, FeedbackDecoder, FeedbackSnapshot,
};

mod bus;
use bus::Bus;

bind_interrupts!(struct Pio1Irqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

const FLAG_ENABLED: u8 = 1 << 0;

const TRACE_SAMPLE_RING_LEN: usize = 16_384;
const COMMAND_RING_LEN: usize = 256;
const CORE1_STACK_SIZE: usize = 4096;

static TRACE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);
static TRACE_QUEUE_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_RXSTALL_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_SAMPLE_RING: StaticCell<Queue<u32, TRACE_SAMPLE_RING_LEN>> = StaticCell::new();
static COMMAND_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static COMMAND_RING: StaticCell<Queue<FeedbackCommand, COMMAND_RING_LEN>> = StaticCell::new();
static mut CORE1_STACK: Stack<CORE1_STACK_SIZE> = Stack::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

pub struct PioTransport {
    trace_samples: Consumer<'static, u32>,
    commands: Consumer<'static, FeedbackCommand>,
    capture_enabled: bool,
    telemetry_enabled: bool,
    packet_seq: u16,
    decoder: FeedbackDecoder,
    current_snapshot: FeedbackSnapshot,
    snapshot_valid: bool,
    telemetry_period_ms: u16,
    next_telemetry_due_ms: u64,
}

impl Transport for PioTransport {
    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                log_info!("handled Ping");
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
                    log_info!(
                        "telemetry_enabled: {} capture_enabled: {}",
                        self.telemetry_enabled,
                        self.capture_enabled
                    );
                }
                1
            }
            MsgType::TelemetrySet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::TelemetrySet as u8, 1);
                } else {
                    self.capture_enabled = req.payload[0] == 0;
                    self.telemetry_enabled = req.payload[0] != 0;
                    TRACE_CAPTURE_ENABLED.store(self.capture_enabled, Ordering::Relaxed);
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
                if self.capture_enabled {
                    out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x10);
                    log_warn!("nacked 0x{:x} (capture enabled)", req.msg_type as u8);
                } else {
                    out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x11);
                    log_warn!("nacked 0x{:x} (capture not enabled)", req.msg_type as u8);
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
            let Some(command) = self.commands.dequeue() else {
                break;
            };

            match self.decoder.ingest_command(command) {
                Ok(s) => {
                    self.current_snapshot = s;
                    self.snapshot_valid = true;
                }
                Err(_e) => {
                    // Keep the last good snapshot.
                }
            }
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
            let timestamp_us = Instant::now().as_micros();
            let pkt = Packet::trace_samples(
                self.packet_seq,
                Some(timestamp_us),
                dropped_samples_total,
                rx_stall_count_total,
                &batch[..used],
            );
            self.packet_seq = self.packet_seq.wrapping_add(1);
            return Some(pkt);
        }

        if self.telemetry_enabled {
            if !self.snapshot_valid {
                log_warn!("snapshot invalid");
                return None;
            }

            if now_ms < self.next_telemetry_due_ms {
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
        self.telemetry_enabled && self.commands.ready()
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
        let (trace_producer, trace_consumer) = trace_ring.split();

        let command_ring = COMMAND_RING.init(Queue::new());
        let (command_producer, command_consumer) = command_ring.split();

        TRACE_CAPTURE_ENABLED.store(false, Ordering::Relaxed);
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);
        COMMAND_DROP_COUNT.store(0, Ordering::Relaxed);

        let capture_pio_resources = unsafe { clone_capture_resources(&pio_resources) };

        embassy_rp::multicore::spawn_core1(
            core1_resources.core1,
            unsafe { &mut *addr_of_mut!(CORE1_STACK) },
            move || {
                let executor1 = EXECUTOR1.init(Executor::new());
                executor1.run(|spawner| {
                    spawner.spawn(
                        core1_loop(pio_resources, command_producer).expect("spawn core1_loop"),
                    );
                    spawner.spawn(
                        capture_core1_loop(capture_pio_resources, trace_producer)
                            .expect("spawn capture_core1_loop"),
                    );
                })
            },
        );

        Self {
            trace_samples: trace_consumer,
            commands: command_consumer,
            capture_enabled: false,
            telemetry_enabled: false,
            packet_seq: 1,
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
            telemetry_period_ms: 10,
            next_telemetry_due_ms: 0,
        }
    }

    fn clear_trace_samples(&mut self) {
        while self.trace_samples.dequeue().is_some() {}
    }

    fn clear_commands(&mut self) {
        while self.commands.dequeue().is_some() {}
    }

    fn reset_stream_state(&mut self) {
        self.packet_seq = 1;
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
        COMMAND_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);
        self.clear_commands();
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

unsafe fn clone_capture_resources(pio_resources: &PioResources) -> PioResources {
    PioResources {
        pio0: pio_resources.pio0.clone_unchecked(),
        pio1: pio_resources.pio1.clone_unchecked(),
        pin_0: pio_resources.pin_0.clone_unchecked(),
        pin_1: pio_resources.pin_1.clone_unchecked(),
        pin_2: pio_resources.pin_2.clone_unchecked(),
        pin_3: pio_resources.pin_3.clone_unchecked(),
        pin_4: pio_resources.pin_4.clone_unchecked(),
        pin_5: pio_resources.pin_5.clone_unchecked(),
        pin_6: pio_resources.pin_6.clone_unchecked(),
        pin_7: pio_resources.pin_7.clone_unchecked(),
        pin_8: pio_resources.pin_8.clone_unchecked(),
        pin_9: pio_resources.pin_9.clone_unchecked(),
        pin_10: pio_resources.pin_10.clone_unchecked(),
        pin_11: pio_resources.pin_11.clone_unchecked(),
        pin_12: pio_resources.pin_12.clone_unchecked(),
        pin_13: pio_resources.pin_13.clone_unchecked(),
        pin_14: pio_resources.pin_14.clone_unchecked(),
        pin_15: pio_resources.pin_15.clone_unchecked(),
        pin_16: pio_resources.pin_16.clone_unchecked(),
        pin_17: pio_resources.pin_17.clone_unchecked(),
        pin_18: pio_resources.pin_18.clone_unchecked(),
        pin_19: pio_resources.pin_19.clone_unchecked(),
        pin_20: pio_resources.pin_20.clone_unchecked(),
        pin_21: pio_resources.pin_21.clone_unchecked(),
        pin_22: pio_resources.pin_22.clone_unchecked(),
        pin_26: pio_resources.pin_26.clone_unchecked(),
        pin_27: pio_resources.pin_27.clone_unchecked(),
        pin_28: pio_resources.pin_28.clone_unchecked(),
    }
}

#[embassy_executor::task]
async fn capture_core1_loop(
    pio_resources: PioResources,
    mut trace_samples: Producer<'static, u32>,
) -> ! {
    let program = pio::pio_file!(
        "../pio/passive_sniffer.pio",
        select_program("fred_passive_sniffer"),
        options(max_program_size = 32)
    );

    let mut pio = Pio::new(pio_resources.pio1, Pio1Irqs);
    let loaded = pio.common.load_program(&program.program);

    let p0 = pio.common.make_pio_pin(pio_resources.pin_0);
    let p1 = pio.common.make_pio_pin(pio_resources.pin_1);
    let p2 = pio.common.make_pio_pin(pio_resources.pin_2);
    let p3 = pio.common.make_pio_pin(pio_resources.pin_3);
    let p4 = pio.common.make_pio_pin(pio_resources.pin_4);
    let p5 = pio.common.make_pio_pin(pio_resources.pin_5);
    let p6 = pio.common.make_pio_pin(pio_resources.pin_6);
    let p7 = pio.common.make_pio_pin(pio_resources.pin_7);
    let p8 = pio.common.make_pio_pin(pio_resources.pin_8);
    let p9 = pio.common.make_pio_pin(pio_resources.pin_9);
    let p10 = pio.common.make_pio_pin(pio_resources.pin_10);
    let p11 = pio.common.make_pio_pin(pio_resources.pin_11);
    let p12 = pio.common.make_pio_pin(pio_resources.pin_12);
    let p13 = pio.common.make_pio_pin(pio_resources.pin_13);
    let p14 = pio.common.make_pio_pin(pio_resources.pin_14);
    let p15 = pio.common.make_pio_pin(pio_resources.pin_15);
    let p16 = pio.common.make_pio_pin(pio_resources.pin_16);
    let p17 = pio.common.make_pio_pin(pio_resources.pin_17);
    let p18 = pio.common.make_pio_pin(pio_resources.pin_18);
    let p27 = pio.common.make_pio_pin(pio_resources.pin_27);

    let in_pins = [
        &p0, &p1, &p2, &p3, &p4, &p5, &p6, &p7, // data bus
        &p8, &p9, &p10, &p11, &p12, &p13, &p14, &p15, // addr bus
        &p16, // 1MHzE
        &p17, // RnW
        &p18, // FRED
    ];

    let mut cfg = Config::default();
    cfg.use_program(&loaded, &[&p27]);
    cfg.set_in_pins(&in_pins);
    cfg.set_jmp_pin(&p18);
    cfg.shift_in = ShiftConfig {
        threshold: 32,
        direction: ShiftDirection::Left,
        auto_fill: false,
    };
    cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 20_000_000);

    pio.sm0.set_config(&cfg);
    pio.sm0.set_pin_dirs(Direction::In, &in_pins);
    pio.sm0.set_pin_dirs(Direction::Out, &[&p27]);
    pio.sm0.clear_fifos();

    let mut batch = PioBatch::new();
    batch.restart(&mut pio.sm0);
    batch.set_enable(&mut pio.sm0, true);
    batch.execute();

    let _ = pio.sm0.rx().stalled();
    log_info!("PIO1 capture initialised on core1");

    loop {
        let mut drained = false;
        while let Some(raw_sample) = pio.sm0.rx().try_pull() {
            drained = true;

            if !TRACE_CAPTURE_ENABLED.load(Ordering::Relaxed) {
                continue;
            }

            let sample = encode_trace_sample(raw_sample);
            if trace_samples.enqueue(sample).is_err() {
                TRACE_QUEUE_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        if pio.sm0.rx().stalled() {
            TRACE_RXSTALL_COUNT.fetch_add(1, Ordering::Relaxed);
        }

        if !drained {
            Timer::after(Duration::from_micros(1)).await;
        }
    }
}

#[inline]
fn encode_trace_sample(raw_sample: u32) -> u32 {
    raw_sample
}

#[embassy_executor::task]
async fn core1_loop(
    pio_resources: PioResources,
    mut commands: Producer<'static, FeedbackCommand>,
) -> ! {
    let mut bus = Bus::setup(pio_resources);

    const CMD_SEQUENCE: [u8; 10] = [0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0D, 0x0C];

    let mut index = 0;
    loop {
        Timer::after(Duration::from_millis(10)).await;
        for cmd in CMD_SEQUENCE {
            let value = bus.command_cycle(cmd).await;
            if commands
                .enqueue(FeedbackCommand::from_bytes(index, cmd, value))
                .is_err()
            {
                COMMAND_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            index += 1;
        }
    }
}
