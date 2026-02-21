use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{Config, Direction, InterruptHandler, Pio, ShiftConfig, ShiftDirection};
use rp_pac as pac;

use crate::resources::SnifferResources;
use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};

bind_interrupts!(struct PioIrqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

pub struct BridgeTransport {
    sniffer: PassiveSniffer,
    capture_enabled: bool,
    trace_seq: u16,
    trace_tick: u32,
}

impl BridgeTransport {
    pub fn new(sniffer_resources: SnifferResources) -> Self {
        let mut sniffer = PassiveSniffer::new();
        sniffer.init(sniffer_resources);
        Self {
            sniffer,
            capture_enabled: true,
            trace_seq: 1,
            trace_tick: 0,
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
        if !self.capture_enabled {
            return None;
        }

        let sample = self.sniffer.poll_sample()?;
        let pkt = Packet::trace_sample(self.trace_seq, self.trace_tick, sample);
        self.trace_seq = self.trace_seq.wrapping_add(1);
        self.trace_tick = self.trace_tick.wrapping_add(1);
        Some(pkt)
    }

    pub fn post_send_delay_ms(&self, _pkt: &Packet) -> Option<u64> {
        None
    }
}

struct PassiveSniffer {
    initialized: bool,
}

impl PassiveSniffer {
    const fn new() -> Self {
        Self { initialized: false }
    }

    fn init(&mut self, r: SnifferResources) {
        if self.initialized {
            return;
        }

        let program = pio::pio_file!(
            "../pio/passive_sniffer.pio",
            select_program("fred_passive_sniffer"),
            options(max_program_size = 32)
        );

        let mut pio = Pio::new(r.pio0, PioIrqs);
        let loaded = pio.common.load_program(&program.program);

        let p0 = pio.common.make_pio_pin(r.pin_0);
        let p1 = pio.common.make_pio_pin(r.pin_1);
        let p2 = pio.common.make_pio_pin(r.pin_2);
        let p3 = pio.common.make_pio_pin(r.pin_3);
        let p4 = pio.common.make_pio_pin(r.pin_4);
        let p5 = pio.common.make_pio_pin(r.pin_5);
        let p6 = pio.common.make_pio_pin(r.pin_6);
        let p7 = pio.common.make_pio_pin(r.pin_7);
        let p8 = pio.common.make_pio_pin(r.pin_8);
        let p9 = pio.common.make_pio_pin(r.pin_9);
        let p10 = pio.common.make_pio_pin(r.pin_10);
        let p11 = pio.common.make_pio_pin(r.pin_11);
        let p12 = pio.common.make_pio_pin(r.pin_12);
        let p13 = pio.common.make_pio_pin(r.pin_13);
        let p14 = pio.common.make_pio_pin(r.pin_14);
        let p15 = pio.common.make_pio_pin(r.pin_15);
        let p16 = pio.common.make_pio_pin(r.pin_16);
        let p17 = pio.common.make_pio_pin(r.pin_17);
        let p18 = pio.common.make_pio_pin(r.pin_18);
        let p19 = pio.common.make_pio_pin(r.pin_19);
        let p20 = pio.common.make_pio_pin(r.pin_20);

        let in_pins = [
            &p0, &p1, &p2, &p3, &p4, &p5, &p6, &p7, &p8, &p9, &p10, &p11, &p12, &p13, &p14, &p15,
            &p16, &p17, &p18, &p19, &p20,
        ];

        let mut cfg = Config::default();
        cfg.use_program(&loaded, &[]);
        cfg.set_in_pins(&in_pins);
        cfg.set_jmp_pin(&p20);
        cfg.shift_in = ShiftConfig {
            threshold: 32,
            direction: ShiftDirection::Right,
            auto_fill: false,
        };

        pio.sm2.set_config(&cfg);
        pio.sm2.set_pin_dirs(Direction::In, &in_pins);
        pio.sm2.clear_fifos();

        pio.common.apply_sm_batch(|batch| {
            batch.restart(&mut pio.sm2);
            batch.set_enable(&mut pio.sm2, true);
        });

        core::mem::forget(pio);
        self.initialized = true;
    }

    #[inline]
    fn poll_sample(&mut self) -> Option<u32> {
        pio0_rx_pull(2)
    }
}

#[inline]
fn pio0_rx_pull(sm: usize) -> Option<u32> {
    let rxempty_mask = 1u8 << sm;
    if pac::PIO0.fstat().read().rxempty() & rxempty_mask != 0 {
        return None;
    }
    Some(pac::PIO0.rxf(sm).read())
}
