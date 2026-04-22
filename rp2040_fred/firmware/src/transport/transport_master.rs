use core::ptr::addr_of_mut;

use embassy_executor::Executor;
use embassy_rp::multicore::Stack;
use embassy_time::{Duration, Timer};
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicU32, Ordering};
use static_cell::StaticCell;

use crate::resources::{Core1Resources, PioResources};
use rp2040_fred_protocol::trace_decode::{
    AxisSnapshot, FeedbackCommand, FeedbackDecoder, FeedbackSnapshot,
};

mod bus;
use bus::Bus;

const FLAG_ENABLED: u8 = 1 << 0;

const COMMAND_RING_LEN: usize = 256;
const CORE1_STACK_SIZE: usize = 4096;

static COMMAND_DROP_COUNT: AtomicU32 = AtomicU32::new(0);
static COMMAND_RING: StaticCell<Queue<FeedbackCommand, COMMAND_RING_LEN>> = StaticCell::new();
static mut CORE1_STACK: Stack<CORE1_STACK_SIZE> = Stack::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

pub struct PioTransport {
    commands: Consumer<'static, FeedbackCommand>,
    telemetry_enabled: bool,
    packet_seq: u16,
    decoder: FeedbackDecoder,
    current_snapshot: FeedbackSnapshot,
    snapshot_valid: bool,
    telemetry_period_ms: u16,
    next_telemetry_due_ms: u64,
}

use crate::transport::Transport;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};

impl Transport for PioTransport {
    fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
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
                out[0] = Packet::nack(req.seq, req.msg_type as u8, 0x11);
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

            if let Some(snapshot) = self.decoder.ingest_command(command) {
                self.current_snapshot = snapshot;
                self.snapshot_valid = true;
            }
            processed += 1;
        }
    }

    fn poll_outgoing_packet(&mut self, now_ms: u64) -> Option<Packet> {
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
        self.telemetry_enabled && self.commands.ready()
    }

    fn has_outgoing_packet(&self, now_ms: u64) -> bool {
        self.telemetry_enabled && self.snapshot_valid && now_ms >= self.next_telemetry_due_ms
    }
}

impl PioTransport {
    pub fn new(core1_resources: Core1Resources, pio_resources: PioResources) -> Self {
        let command_ring = COMMAND_RING.init(Queue::new());
        let (producer, consumer) = command_ring.split();

        COMMAND_DROP_COUNT.store(0, Ordering::Relaxed);

        embassy_rp::multicore::spawn_core1(
            core1_resources.core1,
            unsafe { &mut *addr_of_mut!(CORE1_STACK) },
            move || {
                let executor1 = EXECUTOR1.init(Executor::new());
                executor1.run(|spawner| spawner.spawn(core1_loop(pio_resources, producer).unwrap()))
            },
        );

        Self {
            commands: consumer,
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
            telemetry_period_ms: 100,
            next_telemetry_due_ms: 0,
        }
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
        self.clear_commands();
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
async fn core1_loop(
    pio_resources: PioResources,
    mut commands: Producer<'static, FeedbackCommand>,
) -> ! {
    let mut bus = Bus::setup(pio_resources);

    const CMD_SEQUENCE: [u8; 10] = [0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0D, 0x0C];

    let mut index = 0;
    loop {
        for cmd in CMD_SEQUENCE {
            Timer::after(Duration::from_micros(100)).await;
            let value = bus.command_cycle(cmd).await;
            if commands
                .enqueue(FeedbackCommand::from_bytes(index, cmd, value))
                .is_err()
            {
                COMMAND_DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            index += 1;
        }

        Timer::after(Duration::from_micros(1000)).await;
    }
}
