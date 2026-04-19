use core::hint::spin_loop;
use core::ptr::addr_of_mut;

use embassy_executor::Executor;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Drive, Input, Level, Output, OutputOpenDrain, Pull};
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::peripherals::{PIO0, PIO1};
use embassy_rp::pio::{
    Config, Direction, Instance, InterruptHandler, Pio, PioBatch, ShiftConfig, ShiftDirection, StateMachine
};
use embassy_rp::pio_programs::clock_divider::calculate_pio_clock_divider_value;
use pio::SideSet;
use rp_pac as pac;

use embassy_time::{Duration, Timer};
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use static_cell::StaticCell;

use crate::resources::{Core1Resources, PioResources};
use crate::transport::Transport;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet, TRACE_SAMPLES_PER_PACKET};
use rp2040_fred_protocol::trace_decode::{AxisSnapshot, FeedbackDecoder, FeedbackSnapshot};

macro_rules! log_info {
    ($($arg:tt)*) => {
        defmt::info!($($arg)*);
    };
}

bind_interrupts!(struct Pio0Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});
bind_interrupts!(struct Pio1Irqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

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
}

impl PioTransport {
    pub fn new(core1_resources: Core1Resources, pio_resources: PioResources) -> Self {
        let trace_ring = TRACE_SAMPLE_RING.init(Queue::new());
        let (producer, consumer) = trace_ring.split();

        TRACE_CAPTURE_ENABLED.store(true, Ordering::Relaxed);
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);

        // spawn_core1(
        //     core1_resources.core1,
        //     unsafe { &mut *addr_of_mut!(CORE1_STACK) },
        //     move || core1_loop(pio_resources, producer),
        // );

        embassy_rp::multicore::spawn_core1(core1_resources.core1, unsafe { &mut *addr_of_mut!(CORE1_STACK) }, move || {
            let executor1 = EXECUTOR1.init(Executor::new());
            executor1.run(|spawner| spawner.spawn(core1_loop(pio_resources, producer).unwrap()))
        });

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

#[embassy_executor::task]
async fn core1_loop(pio_resources: PioResources, mut trace_samples: Producer<'static, u32>) -> ! {

    log_info!("running core1_loop");

    // address data-dir high for output.
    let mut data_dir_a = Output::new(pio_resources.pin_26, Level::High);
    data_dir_a.set_high();

    log_info!("set p26 high");

    let fred_bm_clock = pio::pio_file!(
        "../pio/bus_master.pio",
        select_program("fred_bm_clock"),
        options(max_program_size = 32)
    );

    let fred_bm_control = pio::pio_file!(
        "../pio/bus_master.pio",
        select_program("fred_bm_control"),
        options(max_program_size = 32)
    );

    let fred_bm_data_write = pio::pio_file!(
        "../pio/bus_master.pio",
        select_program("fred_bm_data_write"),
        options(max_program_size = 32)
    );

    let fred_bm_data_read = pio::pio_file!(
        "../pio/bus_master.pio",
        select_program("fred_bm_data_read"),
        options(max_program_size = 32)
    );

    log_info!("PIO programs read");

    let mut pio0 = Pio::new(pio_resources.pio0, Pio0Irqs);

    let mut clock_sm = pio0.sm0;
    let mut control_sm = pio0.sm1;
    let mut write_sm = pio0.sm2;
    let mut read_sm = pio0.sm3;

    log_info!("got SMs");

    let clock_program = pio0.common.load_program(&fred_bm_clock.program);
    log_info!("loaded sm0");
    let control_program = pio0.common.load_program(&fred_bm_control.program);
    log_info!("loaded sm1");
    let write_program = pio0.common.load_program(&fred_bm_data_write.program);
    log_info!("loaded sm2");
    let read_program = pio0.common.load_program(&fred_bm_data_read.program);
    log_info!("loaded sm3");

    log_info!("PIO programs loaded");

    let p0 = pio0.common.make_pio_pin(pio_resources.pin_0);
    let p1 = pio0.common.make_pio_pin(pio_resources.pin_1);
    let p2 = pio0.common.make_pio_pin(pio_resources.pin_2);
    let p3 = pio0.common.make_pio_pin(pio_resources.pin_3);
    let p4 = pio0.common.make_pio_pin(pio_resources.pin_4);
    let p5 = pio0.common.make_pio_pin(pio_resources.pin_5);
    let p6 = pio0.common.make_pio_pin(pio_resources.pin_6);
    let p7 = pio0.common.make_pio_pin(pio_resources.pin_7);
    let p8 = pio0.common.make_pio_pin(pio_resources.pin_8);
    let p9 = pio0.common.make_pio_pin(pio_resources.pin_9);
    let p10 = pio0.common.make_pio_pin(pio_resources.pin_10);
    let p11 = pio0.common.make_pio_pin(pio_resources.pin_11);
    let p12 = pio0.common.make_pio_pin(pio_resources.pin_12);
    let p13 = pio0.common.make_pio_pin(pio_resources.pin_13);
    let p14 = pio0.common.make_pio_pin(pio_resources.pin_14);
    let p15 = pio0.common.make_pio_pin(pio_resources.pin_15);
    let p16 = pio0.common.make_pio_pin(pio_resources.pin_16);
    let p17 = pio0.common.make_pio_pin(pio_resources.pin_17);
    let p18 = pio0.common.make_pio_pin(pio_resources.pin_18);
    let p19 = pio0.common.make_pio_pin(pio_resources.pin_19);
    let p20 = pio0.common.make_pio_pin(pio_resources.pin_20);
    // let p21 = pio0.common.make_pio_pin(pio_resources.pin_21);
    let p27 = pio0.common.make_pio_pin(pio_resources.pin_27);
    // let p28 = pio0.common.make_pio_pin(pio_resources.pin_28);

    let data_bus_pins = [&p0, &p1, &p2, &p3, &p4, &p5, &p6, &p7];

    let addr_bus_pins = [&p8, &p9, &p10, &p11, &p12, &p13, &p14, &p15];

    let control_pins = [
        &p17, // RnW
        &p18, // NMI
        &p19, // IRQ
        &p20, // FRED
    ];

    let clock_pin = [
        &p16, // 1MHzE
    ];

    let data_dir_pin = [&p27];

    log_info!("starting PIO pin setup");

    clock_sm.set_pin_dirs(Direction::In, &data_bus_pins);
    clock_sm.set_pin_dirs(Direction::Out, &addr_bus_pins);
    clock_sm.set_pin_dirs(Direction::Out, &data_dir_pin);

    // setting up control pins for open-drain output
    clock_sm.set_pins(Level::Low, &control_pins);
    clock_sm.set_pin_dirs(Direction::In, &control_pins);
    clock_sm.set_pins(Level::Low, &clock_pin);
    clock_sm.set_pin_dirs(Direction::In, &clock_pin);

    clock_sm.clear_fifos();
    control_sm.clear_fifos();
    write_sm.clear_fifos();
    read_sm.clear_fifos();

    let mut clock_cfg = Config::default();
    clock_cfg.use_program(&clock_program, &clock_pin);
    clock_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 500_000);
    clock_sm.set_config(&clock_cfg);

    let mut control_cfg = Config::default();
    control_cfg.use_program(&control_program, &control_pins);
    control_cfg.set_out_pins(&addr_bus_pins);
    control_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 50_000_000);
    control_cfg.shift_out = ShiftConfig {
        threshold: 32,
        direction: ShiftDirection::Left,
        auto_fill: false,
    };
    control_sm.set_config(&control_cfg);

    let mut write_cfg = Config::default();
    write_cfg.use_program(&write_program, &data_dir_pin);
    write_cfg.set_out_pins(&data_bus_pins);
    write_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 10_000_000);
    write_cfg.shift_out = ShiftConfig {
        threshold: 32,
        direction: ShiftDirection::Left,
        auto_fill: false,
    };
    write_sm.set_config(&write_cfg);

    let mut read_cfg = Config::default();
    read_cfg.use_program(&read_program, &[]);
    read_cfg.set_in_pins(&data_bus_pins);
    read_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 20_000_000);
    read_sm.set_config(&read_cfg);

    set_cycle(&mut control_sm, false, false);

    log_info!("enabling SMs");

    let mut batch = PioBatch::new();
    batch.restart(&mut clock_sm);
    batch.set_enable(&mut clock_sm, true);
    batch.restart(&mut control_sm);
    batch.set_enable(&mut control_sm, true);
    batch.restart(&mut write_sm);
    batch.set_enable(&mut write_sm, true);
    batch.restart(&mut read_sm);
    batch.set_enable(&mut read_sm, true);
    batch.execute();

    [16, 17, 18, 19, 20].iter().for_each(|n| {
        pac::PADS_BANK0.gpio(*n).modify(|w| {
            w.set_pue(true);  // pull-up enable
            w.set_pde(false); // pull-down disable
        });
    });
    
    log_info!("PIO initialised on core1");

    loop {
        // write/read loop:
        // command sequence: `03,02,01,00,07,06,05,04,0D,0C`
        // command is:
        // 1. Poll `F0` until bit 0 clears.
        // 2. Write one command byte to `80`.
        // 3. Poll `F0` again until bit 0 clears.
        // 4. Read one response byte from `F1`.

        write_sm.tx().push(0xFFFF_0000u32);   // write payload FF <data> 00 00
        control_sm.tx().push(0xFF00_0000u32); // write

        control_sm.tx().push(0xFF01_0000u32); // read
        let read = read_sm.rx().pull();  // read payload

        Timer::after(Duration::from_micros(500)).await;

    }
}

fn set_write_cycle<P: Instance, const S: usize>(sm: &mut StateMachine<P, S>) {
    set_cycle(sm, true, false);
}
fn set_read_cycle<P: Instance, const S: usize>(sm: &mut StateMachine<P, S>) {
    set_cycle(sm, false, true);
}

fn set_cycle<P: Instance, const S: usize>(sm: &mut StateMachine<P, S>, write: bool, read: bool) {
    unsafe {
        sm.exec_instr(
            pio::Instruction {
                operands: pio::InstructionOperands::SET {
                    destination: pio::SetDestination::X,
                    data: if write { 0 } else { 1 }
                },
                delay: 0,
                side_set: None
            }.encode(SideSet::default())
        );
        sm.exec_instr(
            pio::Instruction {
                operands: pio::InstructionOperands::SET {
                    destination: pio::SetDestination::Y,
                    data: if read { 0 } else { 1 }
                },
                delay: 0,
                side_set: None
            }.encode(SideSet::default())
        );
    }
}