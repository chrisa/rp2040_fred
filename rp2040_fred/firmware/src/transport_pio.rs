use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{
    Config, Direction, InterruptHandler, Pio, PioBatch, ShiftConfig, ShiftDirection,
};
use embassy_rp::pio_programs::clock_divider::calculate_pio_clock_divider_value;
use heapless::spsc::{Consumer, Producer, Queue};
use portable_atomic::{AtomicBool, Ordering};
use static_cell::StaticCell;

use crate::resources::SnifferResources;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet, TRACE_SAMPLES_PER_PACKET};

macro_rules! log_info {
    ($($arg:tt)*) => {
        defmt::info!($($arg)*);
    };
}

bind_interrupts!(struct PioIrqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

const TRACE_SAMPLE_RING_LEN: usize = 2048;
static TRACE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(true);
static TRACE_SAMPLE_RING: StaticCell<Queue<u32, TRACE_SAMPLE_RING_LEN>> = StaticCell::new();

pub struct BridgeTransport {
    trace_samples: Consumer<'static, u32>,
    capture_enabled: bool,
    trace_seq: u16,
}

impl BridgeTransport {
    pub fn new(spawner: &Spawner, sniffer_resources: SnifferResources) -> Self {
        let trace_ring = TRACE_SAMPLE_RING.init(Queue::new());
        let (producer, consumer) = trace_ring.split();

        TRACE_CAPTURE_ENABLED.store(true, Ordering::Relaxed);
        spawner
            .spawn(pio_capture_task(sniffer_resources, producer).expect("create pio capture task"));

        Self {
            trace_samples: consumer,
            capture_enabled: true,
            trace_seq: 1,
        }
    }

    pub fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
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
                    TRACE_CAPTURE_ENABLED.store(self.capture_enabled, Ordering::Relaxed);
                    self.clear_trace_samples();
                    out[0] = Packet::ack(req.seq, MsgType::CaptureSet, 0);
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

    pub fn poll_outgoing_packet(&mut self) -> Option<Packet> {
        if !self.capture_enabled || !self.trace_samples.ready() {
            return None;
        }

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

        let pkt = Packet::trace_samples(self.trace_seq, &batch[..used]);
        self.trace_seq = self.trace_seq.wrapping_add(1);
        Some(pkt)
    }

    pub fn post_send_delay_ms(&self, _pkt: &Packet) -> Option<u64> {
        None
    }

    pub fn has_outgoing_backlog(&self) -> bool {
        self.capture_enabled && self.trace_samples.ready()
    }

    fn clear_trace_samples(&mut self) {
        while self.trace_samples.dequeue().is_some() {}
    }
}

#[embassy_executor::task]
async fn pio_capture_task(
    sniffer_resources: SnifferResources,
    mut trace_samples: Producer<'static, u32>,
) {
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

    log_info!("PIO initialised");

    loop {
        let raw_sample = pio.sm2.rx().wait_pull().await;

        if !TRACE_CAPTURE_ENABLED.load(Ordering::Relaxed) {
            continue;
        }

        let sample = encode_trace_sample(raw_sample);

        // Drop the newest sample on overflow, but keep draining the RX FIFO so
        // the PIO state machine can continue running.
        let _ = trace_samples.enqueue(sample);
    }
}

#[inline]
fn encode_trace_sample(raw_sample: u32) -> u32 {
    // The non-consecutive hardware map keeps bus bits on GPIO0..17 and uses
    // GPIO20 for FRED_N, leaving GPIO18/19 intentionally unused.
    raw_sample
}
