use core::ptr::addr_of_mut;

use embassy_executor::Executor;
use embassy_futures::yield_now;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::multicore::Stack;
use embassy_time::{Duration, Instant, Timer};
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use rp2040_fred_firmware::{log_info, log_warn};
use static_cell::StaticCell;

use crate::decoder::{FeedbackCommand, FeedbackDecoder, FeedbackSnapshot};
use crate::resources::{
    Core1Resources, DebugPin27Resources, DebugPin28Resources, DirectionResources, PioResources,
};
use crate::transport::pio::master::ThisMasterPio;
use crate::transport::pio::passive::PassivePio;

use rp2040_fred_protocol::bridge_proto::{
    CommandBlockRequest, ControllerAction, ControllerStatus, ExperimentBusOp, ExperimentBusOpKind,
    ExperimentBusOpRecord, ExperimentEventKind, ExperimentEventRecord, ExperimentRecord,
    ExperimentRunRequest, ExperimentSampleRecord, ExperimentStatus, MsgType, Packet,
    RpmServiceMode, EXPERIMENT_BUS_OP_STATUS_OK, EXPERIMENT_BUS_OP_STATUS_TIMEOUT,
    EXPERIMENT_STATUS_ACTIVE, EXPERIMENT_STATUS_DONE, EXPERIMENT_STATUS_ERROR,
    EXPERIMENT_STATUS_RECORDS_DROPPED, TELEMETRY_FLAG_COMMAND_ACTIVE,
    TELEMETRY_FLAG_CONTROLLER_BUSY, TELEMETRY_FLAG_CONTROLLER_ERROR, TELEMETRY_FLAG_ENABLED,
    TRACE_SAMPLES_PER_PACKET,
};

mod bus;
use bus::{Bus, CYCLE_START_MASK, PROC_BUSY_MASK};

const TRACE_SAMPLE_RING_LEN: usize = 16_384;
const COMMAND_RING_LEN: usize = 256;
const CONTROLLER_WORK_RING_LEN: usize = 16;
const EXPERIMENT_RECORD_RING_LEN: usize = 512;
const CORE1_STACK_SIZE: usize = 4096;
const PROC_CONT_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_START_TIMEOUT: Duration = Duration::from_secs(5);
const COMMAND_BLOCK_TIMEOUT: Duration = Duration::from_secs(120);
const CAPTURE_IDLE_BURST_SAMPLES: usize = 64;
const CAPTURE_CONTROLLER_WORK_BURST_SAMPLES: usize = 8;

static TRACE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);
static TRACE_QUEUE_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_RXSTALL_COUNT: AtomicU32 = AtomicU32::new(0);
static TRACE_SAMPLE_RING: StaticCell<Queue<u32, TRACE_SAMPLE_RING_LEN>> = StaticCell::new();
static COMMAND_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static COMMAND_RING: StaticCell<Queue<FeedbackCommand, COMMAND_RING_LEN>> = StaticCell::new();
static CONTROLLER_WORK_PENDING_COUNT: AtomicU32 = AtomicU32::new(0);
static POLLING_SUSPEND_PENDING_COUNT: AtomicU32 = AtomicU32::new(0);
static CONTROLLER_WORK_ACTIVE: AtomicBool = AtomicBool::new(false);
static CONTROLLER_BUSY: AtomicBool = AtomicBool::new(false);
static CONTROLLER_ERROR: AtomicBool = AtomicBool::new(false);
static POSITION_POLLING_SUSPENDED: AtomicBool = AtomicBool::new(false);
static RPM_SERVICE_MODE: AtomicU8 = AtomicU8::new(RpmServiceMode::Manual as u8);
static EXPERIMENT_ACTIVE: AtomicBool = AtomicBool::new(false);
static EXPERIMENT_DONE: AtomicBool = AtomicBool::new(false);
static EXPERIMENT_ERROR: AtomicBool = AtomicBool::new(false);
static EXPERIMENT_TRIAL_ID: AtomicU32 = AtomicU32::new(0);
static EXPERIMENT_RECORD_PENDING_COUNT: AtomicU32 = AtomicU32::new(0);
static EXPERIMENT_RECORD_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static CONTROLLER_WORK_RING: StaticCell<Queue<ControllerWork, CONTROLLER_WORK_RING_LEN>> =
    StaticCell::new();
static EXPERIMENT_RECORD_RING: StaticCell<Queue<ExperimentRecord, EXPERIMENT_RECORD_RING_LEN>> =
    StaticCell::new();
static mut CORE1_STACK: Stack<CORE1_STACK_SIZE> = Stack::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

pub struct BusMasterTransport {
    trace_samples: Consumer<'static, u32>,
    commands: Consumer<'static, FeedbackCommand>,
    controller_work: Producer<'static, ControllerWork>,
    experiment_records: Consumer<'static, ExperimentRecord>,
    capture_enabled: bool,
    telemetry_enabled: bool,
    master_packet_seq: u16,
    capture_packet_seq: u16,
    decoder: FeedbackDecoder,
    current_snapshot: FeedbackSnapshot,
    snapshot_valid: bool,
    telemetry_period_ms: u16,
    next_telemetry_due_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControllerWork {
    CycleStartWait,
    CommandBlock(CommandBlockRequest),
    Experiment(ExperimentRunRequest),
}

impl ControllerWork {
    fn suspends_polling(self) -> bool {
        true
    }
}

impl BusMasterTransport {
    pub fn new(
        core1_resources: Core1Resources,
        pio_resources: PioResources,
        dir_resources: DirectionResources,
        debug27_resources: DebugPin27Resources,
        debug28_resources: DebugPin28Resources,
    ) -> Self {
        let trace_ring = TRACE_SAMPLE_RING.init(Queue::new());
        let (trace_producer, trace_consumer) = trace_ring.split();

        let command_ring = COMMAND_RING.init(Queue::new());
        let (command_producer, command_consumer) = command_ring.split();

        let controller_work_ring = CONTROLLER_WORK_RING.init(Queue::new());
        let (controller_work_producer, controller_work_consumer) = controller_work_ring.split();

        let experiment_record_ring = EXPERIMENT_RECORD_RING.init(Queue::new());
        let (experiment_record_producer, experiment_record_consumer) =
            experiment_record_ring.split();

        TRACE_CAPTURE_ENABLED.store(false, Ordering::Relaxed);
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);
        COMMAND_DROP_COUNT.store(0, Ordering::Relaxed);
        CONTROLLER_WORK_PENDING_COUNT.store(0, Ordering::Relaxed);
        POLLING_SUSPEND_PENDING_COUNT.store(0, Ordering::Relaxed);
        CONTROLLER_WORK_ACTIVE.store(false, Ordering::Relaxed);
        CONTROLLER_BUSY.store(false, Ordering::Relaxed);
        CONTROLLER_ERROR.store(false, Ordering::Relaxed);
        POSITION_POLLING_SUSPENDED.store(false, Ordering::Relaxed);
        EXPERIMENT_ACTIVE.store(false, Ordering::Relaxed);
        EXPERIMENT_DONE.store(false, Ordering::Relaxed);
        EXPERIMENT_ERROR.store(false, Ordering::Relaxed);
        EXPERIMENT_TRIAL_ID.store(0, Ordering::Relaxed);
        EXPERIMENT_RECORD_PENDING_COUNT.store(0, Ordering::Relaxed);
        EXPERIMENT_RECORD_DROP_COUNT.store(0, Ordering::Relaxed);

        // SAFETY: capture only reads these pins
        let capture_pio_resources = unsafe { clone_capture_resources(&pio_resources) };

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
                    core1_loop(
                        pio_resources,
                        dir_resources,
                        debug28_resources,
                        command_producer,
                        controller_work_consumer,
                        experiment_record_producer,
                    )
                    .expect("spawn core1_loop"),
                );
                spawner.spawn(
                    capture_core1_loop(capture_pio_resources, debug27_resources, trace_producer)
                        .expect("spawn capture_core1_loop"),
                );
            })
        });

        Self {
            trace_samples: trace_consumer,
            commands: command_consumer,
            controller_work: controller_work_producer,
            experiment_records: experiment_record_consumer,
            capture_enabled: false,
            telemetry_enabled: false,
            master_packet_seq: 1,
            capture_packet_seq: 1,
            decoder: FeedbackDecoder::new(),
            current_snapshot: FeedbackSnapshot::default(),
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

    fn clear_experiment_records(&mut self) {
        while self.experiment_records.dequeue().is_some() {
            EXPERIMENT_RECORD_PENDING_COUNT.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn handle_master_request(&mut self, req: &Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                log_info!("handled master Ping");
                1
            }
            MsgType::TelemetrySet => {
                if req.payload_len < 4 {
                    out[0] = Packet::nack(req.seq, MsgType::TelemetrySet as u8, 1);
                } else {
                    let Some(rpm_service_mode) = RpmServiceMode::from_u8(req.payload[3]) else {
                        out[0] = Packet::nack(req.seq, MsgType::TelemetrySet as u8, 2);
                        return 1;
                    };

                    self.telemetry_enabled = req.payload[0] != 0;
                    self.reset_master_stream_state();
                    self.telemetry_period_ms = u16::from_le_bytes([req.payload[1], req.payload[2]]);
                    RPM_SERVICE_MODE.store(rpm_service_mode as u8, Ordering::Relaxed);
                    out[0] = Packet::ack(req.seq, MsgType::TelemetrySet, 0);
                    log_info!(
                        "telemetry_enabled: {} rpm_service_mode: {}",
                        self.telemetry_enabled,
                        rpm_service_mode as u8
                    );
                }
                1
            }
            MsgType::CommandBlock => {
                let Some(request) = req.decode_command_block_request() else {
                    out[0] = Packet::nack(req.seq, MsgType::CommandBlock as u8, 1);
                    return 1;
                };

                let work = ControllerWork::CommandBlock(request);
                if self.controller_work.enqueue(work).is_err() {
                    out[0] = Packet::nack(req.seq, MsgType::CommandBlock as u8, 0x12);
                    log_warn!("nacked CommandBlock (queue full)");
                    return 1;
                }

                enqueue_controller_work(work.suspends_polling());
                if work.suspends_polling() {
                    self.reset_position_polling_state();
                }
                out[0] = Packet::ack(req.seq, MsgType::CommandBlock, 0);
                log_info!("queued CommandBlock");
                1
            }
            MsgType::ControllerAction => {
                let Some(action_request) = req.decode_controller_action_request() else {
                    out[0] = Packet::nack(req.seq, MsgType::ControllerAction as u8, 1);
                    return 1;
                };

                let work = match action_request.action {
                    ControllerAction::CycleStartWait => ControllerWork::CycleStartWait,
                };

                if self.controller_work.enqueue(work).is_err() {
                    out[0] = Packet::nack(req.seq, MsgType::ControllerAction as u8, 0x12);
                    log_warn!("nacked ControllerAction (queue full)");
                    return 1;
                }

                enqueue_controller_work(work.suspends_polling());
                if work.suspends_polling() {
                    self.reset_position_polling_state();
                }
                out[0] = Packet::ack(req.seq, MsgType::ControllerAction, 0);
                log_info!("queued ControllerAction");
                1
            }
            MsgType::ControllerStatusReq => {
                out[0] = Packet::controller_status_ack(req.seq, self.controller_status());
                1
            }
            MsgType::ExperimentRun => {
                let Some(request) = req.decode_experiment_run() else {
                    out[0] = Packet::nack(req.seq, MsgType::ExperimentRun as u8, 1);
                    return 1;
                };

                if request.feedback_period_ms == 0 {
                    out[0] = Packet::nack(req.seq, MsgType::ExperimentRun as u8, 2);
                    return 1;
                }

                let work = ControllerWork::Experiment(request);
                if self.controller_work.enqueue(work).is_err() {
                    out[0] = Packet::nack(req.seq, MsgType::ExperimentRun as u8, 0x12);
                    log_warn!("nacked ExperimentRun (queue full)");
                    return 1;
                }

                self.clear_experiment_records();
                EXPERIMENT_ACTIVE.store(true, Ordering::Relaxed);
                EXPERIMENT_DONE.store(false, Ordering::Relaxed);
                EXPERIMENT_ERROR.store(false, Ordering::Relaxed);
                EXPERIMENT_TRIAL_ID.store(request.trial_id, Ordering::Relaxed);
                EXPERIMENT_RECORD_DROP_COUNT.store(0, Ordering::Relaxed);
                enqueue_controller_work(work.suspends_polling());
                if work.suspends_polling() {
                    self.reset_position_polling_state();
                }
                out[0] = Packet::ack(req.seq, MsgType::ExperimentRun, 0);
                log_info!("queued ExperimentRun");
                1
            }
            MsgType::ExperimentStatusReq => {
                out[0] = Packet::experiment_status_ack(req.seq, self.experiment_status());
                1
            }
            _ => {
                out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x21);
                log_warn!("nacked master request 0x{:x}", req.msg_type as u8);
                1
            }
        }
    }

    pub fn handle_capture_request(&mut self, req: &Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                log_info!("handled capture Ping");
                1
            }
            MsgType::CaptureSet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::CaptureSet as u8, 1);
                } else {
                    self.capture_enabled = req.payload[0] != 0;
                    TRACE_CAPTURE_ENABLED.store(self.capture_enabled, Ordering::Relaxed);
                    self.reset_capture_stream_state();
                    out[0] = Packet::ack(req.seq, MsgType::CaptureSet, 0);
                    log_info!("capture_enabled: {}", self.capture_enabled);
                }
                1
            }
            _ => {
                out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x22);
                log_warn!("nacked capture request 0x{:x}", req.msg_type as u8);
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

    pub fn poll_master_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet> {
        if let Some(record) = self.experiment_records.dequeue() {
            EXPERIMENT_RECORD_PENDING_COUNT.fetch_sub(1, Ordering::Relaxed);
            let pkt = match record {
                ExperimentRecord::Sample(record) => {
                    Packet::experiment_sample(self.master_packet_seq, record)
                }
                ExperimentRecord::BusOp(record) => {
                    Packet::experiment_bus_op(self.master_packet_seq, record)
                }
                ExperimentRecord::Event(record) => {
                    Packet::experiment_event(self.master_packet_seq, record)
                }
            };
            self.master_packet_seq = self.master_packet_seq.wrapping_add(1);
            return Some(pkt);
        }

        if !self.telemetry_enabled {
            return None;
        }

        if !self.snapshot_valid {
            log_warn!("snapshot invalid");
            return None;
        }

        if now_ms < self.next_telemetry_due_ms {
            return None;
        }

        let pkt = Packet::telemetry(
            self.master_packet_seq,
            0,
            self.current_snapshot.x.count(),
            self.current_snapshot.z.count(),
            self.current_snapshot.s.rpm(),
            self.flags(),
        );
        self.master_packet_seq = self.master_packet_seq.wrapping_add(1);
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
            self.capture_packet_seq,
            Some(timestamp_us),
            dropped_samples_total,
            rx_stall_count_total,
            &batch[..used],
        );
        self.capture_packet_seq = self.capture_packet_seq.wrapping_add(1);
        Some(pkt)
    }

    pub fn has_master_decode_work(&self) -> bool {
        self.telemetry_enabled && self.commands.ready()
    }

    pub fn has_master_outgoing_packet(&self, now_ms: u64) -> bool {
        self.experiment_records.ready()
            || (self.telemetry_enabled
                && self.snapshot_valid
                && now_ms >= self.next_telemetry_due_ms)
    }

    pub fn has_capture_outgoing_packet(&self) -> bool {
        self.capture_enabled && self.trace_samples.ready()
    }

    fn reset_master_stream_state(&mut self) {
        self.master_packet_seq = 1;
        self.decoder = FeedbackDecoder::new();
        self.current_snapshot = FeedbackSnapshot::default();
        self.snapshot_valid = false;
        self.next_telemetry_due_ms = 0;
        COMMAND_DROP_COUNT.store(0, Ordering::Relaxed);
        self.clear_commands();
    }

    fn reset_capture_stream_state(&mut self) {
        self.capture_packet_seq = 1;
        TRACE_QUEUE_DROP_COUNT.store(0, Ordering::Relaxed);
        TRACE_RXSTALL_COUNT.store(0, Ordering::Relaxed);
        self.clear_trace_samples();
    }

    fn reset_position_polling_state(&mut self) {
        self.decoder = FeedbackDecoder::new();
        self.current_snapshot = FeedbackSnapshot::default();
        self.snapshot_valid = false;
        self.next_telemetry_due_ms = 0;
        COMMAND_DROP_COUNT.store(0, Ordering::Relaxed);
        self.clear_commands();
    }

    fn controller_status(&self) -> ControllerStatus {
        ControllerStatus {
            flags: self.flags(),
            pending_count: CONTROLLER_WORK_PENDING_COUNT.load(Ordering::Relaxed),
        }
    }

    fn experiment_status(&self) -> ExperimentStatus {
        let mut flags = 0;
        if EXPERIMENT_ACTIVE.load(Ordering::Relaxed) {
            flags |= EXPERIMENT_STATUS_ACTIVE;
        }
        if EXPERIMENT_DONE.load(Ordering::Relaxed) {
            flags |= EXPERIMENT_STATUS_DONE;
        }
        if EXPERIMENT_ERROR.load(Ordering::Relaxed) {
            flags |= EXPERIMENT_STATUS_ERROR;
        }
        if EXPERIMENT_RECORD_DROP_COUNT.load(Ordering::Relaxed) != 0 {
            flags |= EXPERIMENT_STATUS_RECORDS_DROPPED;
        }

        ExperimentStatus {
            flags,
            pending_records: EXPERIMENT_RECORD_PENDING_COUNT.load(Ordering::Relaxed),
            dropped_records: EXPERIMENT_RECORD_DROP_COUNT.load(Ordering::Relaxed),
            active_trial_id: EXPERIMENT_TRIAL_ID.load(Ordering::Relaxed),
        }
    }

    fn flags(&self) -> u8 {
        let mut flags = 0;
        if self.telemetry_enabled {
            flags |= TELEMETRY_FLAG_ENABLED;
        }
        if CONTROLLER_BUSY.load(Ordering::Relaxed) {
            flags |= TELEMETRY_FLAG_CONTROLLER_BUSY;
        }
        if CONTROLLER_ERROR.load(Ordering::Relaxed) {
            flags |= TELEMETRY_FLAG_CONTROLLER_ERROR;
        }
        if CONTROLLER_WORK_PENDING_COUNT.load(Ordering::Relaxed) != 0
            || CONTROLLER_WORK_ACTIVE.load(Ordering::Relaxed)
        {
            flags |= TELEMETRY_FLAG_COMMAND_ACTIVE;
        }
        flags
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
    }
}

#[embassy_executor::task]
async fn capture_core1_loop(
    pio_resources: PioResources,
    debug_resources: DebugPin27Resources,
    mut trace_samples: Producer<'static, u32>,
) -> ! {
    let mut pio = PassivePio::setup(pio_resources, debug_resources.pin);
    let mut burst_samples = 0_usize;

    loop {
        if !TRACE_CAPTURE_ENABLED.load(Ordering::Relaxed) {
            burst_samples = 0;
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

        burst_samples += 1;
        let burst_limit = if controller_work_in_flight() {
            CAPTURE_CONTROLLER_WORK_BURST_SAMPLES
        } else {
            CAPTURE_IDLE_BURST_SAMPLES
        };
        if burst_samples >= burst_limit {
            burst_samples = 0;
            yield_now().await;
        }
    }
}

const CMD_SEQUENCE: [u8; 10] = [0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0D, 0x0C];
const MANUAL_RPM_TRIGGER_ADDR: u8 = 0x88;
const REMOTE_SPEED_SERVICE_ADDR: u8 = 0xAD;
const RPM_TRIGGER_LOOP_INTERVAL: u8 = 30;
const FEEDBACK_SERVICE_INTERVAL: Duration = Duration::from_millis(10);

#[embassy_executor::task]
async fn core1_loop(
    pio_resources: PioResources,
    dir_resources: DirectionResources,
    debug_resources: DebugPin28Resources,
    mut commands: Producer<'static, FeedbackCommand>,
    mut controller_work: Consumer<'static, ControllerWork>,
    mut experiment_records: Producer<'static, ExperimentRecord>,
) -> ! {
    let pio = ThisMasterPio::setup(pio_resources, dir_resources.pin_19, debug_resources.pin);
    let mut bus = Bus { pio };
    let mut index = 0;
    let mut rpm_trigger_countdown = RPM_TRIGGER_LOOP_INTERVAL;

    // address data-dir high for output.
    let mut dir_a = Output::new(dir_resources.pin_20, Level::High);
    dir_a.set_high();

    // control data-dir high for output
    let mut dir_c = Output::new(dir_resources.pin_21, Level::High);
    dir_c.set_high();

    loop {
        if let Some(work) = controller_work.dequeue() {
            let suspends_polling = work.suspends_polling();
            decrement_controller_work_pending_count();
            if suspends_polling {
                decrement_polling_suspend_pending_count();
                POSITION_POLLING_SUSPENDED.store(true, Ordering::Relaxed);
            }
            CONTROLLER_WORK_ACTIVE.store(true, Ordering::Relaxed);
            CONTROLLER_BUSY.store(true, Ordering::Relaxed);
            match work {
                ControllerWork::CycleStartWait => {
                    if !wait_cycle_start(
                        &mut bus,
                        &mut commands,
                        &mut index,
                        &mut rpm_trigger_countdown,
                        PROC_CONT_TIMEOUT,
                    )
                    .await
                    {
                        CONTROLLER_ERROR.store(true, Ordering::Relaxed);
                        log_warn!("cycle-start wait timed out");
                    }
                }
                ControllerWork::CommandBlock(request) => {
                    let mut run_block = true;
                    if request.cycle_start_wait() {
                        run_block = wait_cycle_start(
                            &mut bus,
                            &mut commands,
                            &mut index,
                            &mut rpm_trigger_countdown,
                            PROC_CONT_TIMEOUT,
                        )
                        .await;
                        if !run_block {
                            CONTROLLER_ERROR.store(true, Ordering::Relaxed);
                            log_warn!("cycle-start wait timed out before CommandBlock");
                        }
                    }
                    if run_block {
                        bus.write_command_block(request.block).await;
                        if wait_command_complete(
                            &mut bus,
                            &mut commands,
                            &mut index,
                            &mut rpm_trigger_countdown,
                            COMMAND_BLOCK_TIMEOUT,
                        )
                        .await
                        {
                            bus.clear_command_block().await;
                        } else {
                            CONTROLLER_ERROR.store(true, Ordering::Relaxed);
                            log_warn!("CommandBlock busy wait timed out");
                        }
                    }
                }
                ControllerWork::Experiment(request) => {
                    if !run_experiment(
                        &mut bus,
                        &mut commands,
                        &mut experiment_records,
                        &mut index,
                        &mut rpm_trigger_countdown,
                        request,
                    )
                    .await
                    {
                        CONTROLLER_ERROR.store(true, Ordering::Relaxed);
                        log_warn!("ExperimentRun failed");
                    }
                }
            }
            CONTROLLER_BUSY.store(false, Ordering::Relaxed);
            CONTROLLER_WORK_ACTIVE.store(false, Ordering::Relaxed);
            if POLLING_SUSPEND_PENDING_COUNT.load(Ordering::Relaxed) == 0 {
                POSITION_POLLING_SUSPENDED.store(false, Ordering::Relaxed);
            }
            continue;
        }

        Timer::after(FEEDBACK_SERVICE_INTERVAL).await;
        if !POSITION_POLLING_SUSPENDED.load(Ordering::Relaxed) {
            poll_feedback_once(
                &mut bus,
                &mut commands,
                &mut index,
                &mut rpm_trigger_countdown,
            )
            .await;
        }
    }
}

async fn wait_cycle_start(
    bus: &mut Bus<'_>,
    commands: &mut Producer<'static, FeedbackCommand>,
    index: &mut u64,
    rpm_trigger_countdown: &mut u8,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        if bus.read_status().await & CYCLE_START_MASK == 0 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }

        if !poll_feedback_once_deadline(bus, commands, index, rpm_trigger_countdown, deadline).await
        {
            return false;
        }
        Timer::after(FEEDBACK_SERVICE_INTERVAL).await;
    }
}

async fn wait_command_complete(
    bus: &mut Bus<'_>,
    commands: &mut Producer<'static, FeedbackCommand>,
    index: &mut u64,
    rpm_trigger_countdown: &mut u8,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        if !poll_feedback_once_deadline(bus, commands, index, rpm_trigger_countdown, deadline).await
        {
            return false;
        }

        if bus.read_status().await & PROC_BUSY_MASK == 0 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }

        Timer::after(FEEDBACK_SERVICE_INTERVAL).await;
    }
}

async fn run_experiment(
    bus: &mut Bus<'_>,
    commands: &mut Producer<'static, FeedbackCommand>,
    experiment_records: &mut Producer<'static, ExperimentRecord>,
    index: &mut u64,
    rpm_trigger_countdown: &mut u8,
    request: ExperimentRunRequest,
) -> bool {
    EXPERIMENT_ACTIVE.store(true, Ordering::Relaxed);
    EXPERIMENT_DONE.store(false, Ordering::Relaxed);
    EXPERIMENT_ERROR.store(false, Ordering::Relaxed);
    EXPERIMENT_TRIAL_ID.store(request.trial_id, Ordering::Relaxed);

    let mut ok = true;
    if request.command.cycle_start_wait()
        && !wait_cycle_start(
            bus,
            commands,
            index,
            rpm_trigger_countdown,
            PROC_CONT_TIMEOUT,
        )
        .await
    {
        ok = false;
    }

    if ok {
        bus.write_command_block(request.command.block).await;
        enqueue_experiment_record(
            experiment_records,
            ExperimentRecord::Event(ExperimentEventRecord {
                trial_id: request.trial_id,
                timestamp_us: Instant::now().as_micros(),
                event: ExperimentEventKind::CommandLoaded,
                status: 0,
                flags: current_controller_flags(),
            }),
        );

        ok =
            run_experiment_busy_loop(bus, experiment_records, rpm_trigger_countdown, request).await;
        if ok {
            bus.clear_command_block().await;
            service_rpm_update(bus, rpm_trigger_countdown).await;
            enqueue_experiment_record(
                experiment_records,
                ExperimentRecord::Event(ExperimentEventRecord {
                    trial_id: request.trial_id,
                    timestamp_us: Instant::now().as_micros(),
                    event: ExperimentEventKind::CommandComplete,
                    status: 0,
                    flags: current_controller_flags(),
                }),
            );
        }
    }

    if !ok {
        EXPERIMENT_ERROR.store(true, Ordering::Relaxed);
        enqueue_experiment_record(
            experiment_records,
            ExperimentRecord::Event(ExperimentEventRecord {
                trial_id: request.trial_id,
                timestamp_us: Instant::now().as_micros(),
                event: ExperimentEventKind::Error,
                status: 1,
                flags: current_controller_flags(),
            }),
        );
    }

    EXPERIMENT_ACTIVE.store(false, Ordering::Relaxed);
    EXPERIMENT_DONE.store(true, Ordering::Relaxed);
    ok
}

async fn run_experiment_busy_loop(
    bus: &mut Bus<'_>,
    experiment_records: &mut Producer<'static, ExperimentRecord>,
    rpm_trigger_countdown: &mut u8,
    request: ExperimentRunRequest,
) -> bool {
    let deadline = Instant::now() + COMMAND_BLOCK_TIMEOUT;
    let start_deadline = Instant::now() + COMMAND_START_TIMEOUT;
    let feedback_period = Duration::from_millis(u64::from(request.feedback_period_ms.max(1)));
    let mut next_feedback_due = Instant::now();
    let mut next_script_due = Instant::now();
    let mut script_index = 0_usize;
    let mut decoder = FeedbackDecoder::new();
    let mut feedback_index = 0_u64;
    let mut observed_busy = false;

    loop {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }

        while script_index < request.script_len as usize && now >= next_script_due {
            let op = request.script[script_index];
            if op.kind == ExperimentBusOpKind::DelayUs {
                enqueue_experiment_record(
                    experiment_records,
                    ExperimentRecord::BusOp(ExperimentBusOpRecord {
                        trial_id: request.trial_id,
                        timestamp_us: Instant::now().as_micros(),
                        op_index: script_index as u8,
                        op_kind: op.kind,
                        status: EXPERIMENT_BUS_OP_STATUS_OK,
                        addr: op.addr,
                        write_value: op.value,
                        read_value: 0,
                    }),
                );
                next_script_due = Instant::now() + Duration::from_micros(u64::from(op.arg_us));
                script_index += 1;
                break;
            }

            execute_experiment_bus_op(
                bus,
                experiment_records,
                &mut decoder,
                &mut feedback_index,
                rpm_trigger_countdown,
                request.trial_id,
                script_index as u8,
                op,
            )
            .await;
            script_index += 1;
        }

        let status = bus.read_status().await;
        if status & PROC_BUSY_MASK != 0 {
            observed_busy = true;
        } else if observed_busy {
            return true;
        } else if now >= start_deadline {
            return false;
        }

        if now >= next_feedback_due {
            poll_experiment_feedback_once(
                bus,
                experiment_records,
                &mut decoder,
                &mut feedback_index,
                rpm_trigger_countdown,
                request.trial_id,
            )
            .await;

            let status = bus.read_status().await;
            if status & PROC_BUSY_MASK != 0 {
                observed_busy = true;
            } else if observed_busy {
                return true;
            }
            next_feedback_due = Instant::now() + feedback_period;
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

async fn execute_experiment_bus_op(
    bus: &mut Bus<'_>,
    experiment_records: &mut Producer<'static, ExperimentRecord>,
    decoder: &mut FeedbackDecoder,
    feedback_index: &mut u64,
    rpm_trigger_countdown: &mut u8,
    trial_id: u32,
    op_index: u8,
    op: ExperimentBusOp,
) {
    let mut status = EXPERIMENT_BUS_OP_STATUS_OK;
    let mut read_value = 0_u8;

    match op.kind {
        ExperimentBusOpKind::DelayUs => {}
        ExperimentBusOpKind::Read => {
            read_value = bus.read_cycle(op.addr).await;
        }
        ExperimentBusOpKind::Write => {
            bus.write_cycle(op.addr, op.value).await;
        }
        ExperimentBusOpKind::WriteGated => {
            bus.write_register_gated(op.addr, op.value).await;
        }
        ExperimentBusOpKind::ReadUntil => {
            let deadline = Instant::now() + Duration::from_micros(u64::from(op.arg_us));
            match bus
                .read_until_mask_value_deadline(op.addr, op.mask, op.match_value, deadline)
                .await
            {
                Some(value) => read_value = value,
                None => status = EXPERIMENT_BUS_OP_STATUS_TIMEOUT,
            }
        }
        ExperimentBusOpKind::PollFeedbackOnce => {
            poll_experiment_feedback_once(
                bus,
                experiment_records,
                decoder,
                feedback_index,
                rpm_trigger_countdown,
                trial_id,
            )
            .await;
        }
    }

    enqueue_experiment_record(
        experiment_records,
        ExperimentRecord::BusOp(ExperimentBusOpRecord {
            trial_id,
            timestamp_us: Instant::now().as_micros(),
            op_index,
            op_kind: op.kind,
            status,
            addr: op.addr,
            write_value: op.value,
            read_value,
        }),
    );
}

async fn poll_experiment_feedback_once(
    bus: &mut Bus<'_>,
    experiment_records: &mut Producer<'static, ExperimentRecord>,
    decoder: &mut FeedbackDecoder,
    feedback_index: &mut u64,
    rpm_trigger_countdown: &mut u8,
    trial_id: u32,
) {
    let mut latest_sample = None;

    for cmd in CMD_SEQUENCE {
        Timer::after(Duration::from_nanos(50)).await;
        let value = bus.command_cycle(cmd).await;
        if let Ok(snapshot) = decoder.ingest_command(FeedbackCommand::from_master(
            *feedback_index,
            cmd,
            value,
            *rpm_trigger_countdown <= 1,
        )) {
            latest_sample = Some(snapshot);
        }
        *feedback_index = (*feedback_index).wrapping_add(1);
    }

    poll_rpm_trigger(bus, rpm_trigger_countdown).await;

    if let Some(snapshot) = latest_sample {
        enqueue_experiment_record(
            experiment_records,
            ExperimentRecord::Sample(ExperimentSampleRecord {
                trial_id,
                timestamp_us: Instant::now().as_micros(),
                sample_index: snapshot.sample_index,
                x_counts: snapshot.x.count(),
                z_counts: snapshot.z.count(),
                rpm: snapshot.s.rpm(),
                flags: current_controller_flags(),
            }),
        );
    }
}

fn enqueue_experiment_record(
    experiment_records: &mut Producer<'static, ExperimentRecord>,
    record: ExperimentRecord,
) {
    if experiment_records.enqueue(record).is_err() {
        EXPERIMENT_RECORD_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    } else {
        EXPERIMENT_RECORD_PENDING_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

async fn poll_feedback_once(
    bus: &mut Bus<'_>,
    commands: &mut Producer<'static, FeedbackCommand>,
    index: &mut u64,
    rpm_trigger_countdown: &mut u8,
) {
    for cmd in CMD_SEQUENCE {
        Timer::after(Duration::from_nanos(50)).await;
        let value = bus.command_cycle(cmd).await;
        enqueue_feedback_command(commands, index, cmd, value, *rpm_trigger_countdown <= 1);
    }

    poll_rpm_trigger(bus, rpm_trigger_countdown).await;
}

async fn poll_feedback_once_deadline(
    bus: &mut Bus<'_>,
    commands: &mut Producer<'static, FeedbackCommand>,
    index: &mut u64,
    rpm_trigger_countdown: &mut u8,
    deadline: Instant,
) -> bool {
    for cmd in CMD_SEQUENCE {
        Timer::after(Duration::from_nanos(50)).await;
        let Some(value) = bus.command_cycle_deadline(cmd, deadline).await else {
            return false;
        };
        enqueue_feedback_command(commands, index, cmd, value, *rpm_trigger_countdown <= 1);
    }

    poll_rpm_trigger_deadline(bus, rpm_trigger_countdown, deadline).await
}

fn enqueue_feedback_command(
    commands: &mut Producer<'static, FeedbackCommand>,
    index: &mut u64,
    cmd: u8,
    value: u8,
    rpm_trigger: bool,
) {
    if commands
        .enqueue(FeedbackCommand::from_master(
            *index,
            cmd,
            value,
            rpm_trigger,
        ))
        .is_err()
    {
        COMMAND_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    *index = (*index).wrapping_add(1);
}

async fn poll_rpm_trigger(bus: &mut Bus<'_>, rpm_trigger_countdown: &mut u8) {
    if *rpm_trigger_countdown <= 1 {
        service_rpm_update(bus, rpm_trigger_countdown).await;
    } else {
        *rpm_trigger_countdown -= 1;
    }
}

async fn poll_rpm_trigger_deadline(
    bus: &mut Bus<'_>,
    rpm_trigger_countdown: &mut u8,
    deadline: Instant,
) -> bool {
    if *rpm_trigger_countdown <= 1 {
        service_rpm_update_deadline(bus, rpm_trigger_countdown, deadline).await
    } else {
        *rpm_trigger_countdown -= 1;
        true
    }
}

async fn service_rpm_update(bus: &mut Bus<'_>, rpm_trigger_countdown: &mut u8) {
    Timer::after(Duration::from_nanos(50)).await;
    match current_rpm_service_mode() {
        RpmServiceMode::Manual => bus.read_write_zero_pair(MANUAL_RPM_TRIGGER_ADDR).await,
        RpmServiceMode::Remote => {
            bus.write_zero_register_gated(REMOTE_SPEED_SERVICE_ADDR)
                .await
        }
    }
    *rpm_trigger_countdown = RPM_TRIGGER_LOOP_INTERVAL;
}

async fn service_rpm_update_deadline(
    bus: &mut Bus<'_>,
    rpm_trigger_countdown: &mut u8,
    deadline: Instant,
) -> bool {
    Timer::after(Duration::from_nanos(50)).await;
    let serviced = match current_rpm_service_mode() {
        RpmServiceMode::Manual => {
            bus.read_write_zero_pair(MANUAL_RPM_TRIGGER_ADDR).await;
            true
        }
        RpmServiceMode::Remote => {
            bus.write_zero_register_gated_deadline(REMOTE_SPEED_SERVICE_ADDR, deadline)
                .await
        }
    };

    if serviced {
        *rpm_trigger_countdown = RPM_TRIGGER_LOOP_INTERVAL;
    }
    serviced
}

fn current_rpm_service_mode() -> RpmServiceMode {
    RpmServiceMode::from_u8(RPM_SERVICE_MODE.load(Ordering::Relaxed))
        .unwrap_or(RpmServiceMode::Manual)
}

fn current_controller_flags() -> u8 {
    let mut flags = 0;
    if CONTROLLER_BUSY.load(Ordering::Relaxed) {
        flags |= TELEMETRY_FLAG_CONTROLLER_BUSY;
    }
    if CONTROLLER_ERROR.load(Ordering::Relaxed) {
        flags |= TELEMETRY_FLAG_CONTROLLER_ERROR;
    }
    if CONTROLLER_WORK_PENDING_COUNT.load(Ordering::Relaxed) != 0
        || CONTROLLER_WORK_ACTIVE.load(Ordering::Relaxed)
    {
        flags |= TELEMETRY_FLAG_COMMAND_ACTIVE;
    }
    flags
}

fn enqueue_controller_work(suspends_polling: bool) {
    CONTROLLER_ERROR.store(false, Ordering::Relaxed);
    CONTROLLER_WORK_PENDING_COUNT.fetch_add(1, Ordering::Relaxed);
    if suspends_polling {
        POLLING_SUSPEND_PENDING_COUNT.fetch_add(1, Ordering::Relaxed);
        POSITION_POLLING_SUSPENDED.store(true, Ordering::Relaxed);
    }
}

fn controller_work_in_flight() -> bool {
    CONTROLLER_WORK_PENDING_COUNT.load(Ordering::Relaxed) != 0
        || CONTROLLER_WORK_ACTIVE.load(Ordering::Relaxed)
}

fn decrement_controller_work_pending_count() {
    loop {
        let count = CONTROLLER_WORK_PENDING_COUNT.load(Ordering::Relaxed);
        if count == 0 {
            return;
        }

        if CONTROLLER_WORK_PENDING_COUNT
            .compare_exchange_weak(count, count - 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
    }
}

fn decrement_polling_suspend_pending_count() {
    loop {
        let count = POLLING_SUSPEND_PENDING_COUNT.load(Ordering::Relaxed);
        if count == 0 {
            return;
        }

        if POLLING_SUSPEND_PENDING_COUNT
            .compare_exchange_weak(count, count - 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
    }
}
