#![allow(dead_code)]

#[cfg(feature = "pio-real")]
use embassy_rp::Peri;
#[cfg(not(test))]
use embassy_rp::Peripherals;

#[cfg(feature = "pio-real")]
use embassy_rp::gpio::Level;
#[cfg(feature = "pio-real")]
use embassy_rp::peripherals::{
    PIN_0, PIN_1, PIN_10, PIN_11, PIN_12, PIN_13, PIN_14, PIN_15, PIN_16, PIN_17, PIN_2, PIN_20,
    PIN_27, PIN_28, PIN_3, PIN_4, PIN_5, PIN_6, PIN_7, PIN_8, PIN_9, PIO0,
};
#[cfg(feature = "pio-real")]
use embassy_rp::pio::{Config, Direction, InterruptHandler, Pio, ShiftConfig, ShiftDirection};
#[cfg(feature = "pio-real")]
use rp_pac as pac;

#[cfg(feature = "pio-real")]
use embassy_rp::bind_interrupts;

use crate::pins::reg;
#[cfg(feature = "mock-bus")]
use rp2040_fred_protocol::protocol::FredReply;

#[cfg(all(feature = "mock-bus", feature = "pio-real"))]
compile_error!("Use either `mock-bus` or `pio-real`, not both.");

#[cfg(feature = "pio-real")]
bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

/// Bus-master transport shell for RP2040 1MHz-bus generation.
///
/// Target pins/signals this module will drive:
/// - A0..A7
/// - D0..D7 (shared bidirectional bus via external transceiver)
/// - 1MHZE
/// - RnW
/// - FRED_N (predecoded FRED select)
/// - DATA_DIR / DATA_OE_N
pub struct Rp2040FredTransport {
    #[cfg(feature = "mock-bus")]
    status_fcf0: u8,
    #[cfg(feature = "mock-bus")]
    response_fcf1: u8,

    last_write_word: u16,
    last_read_word: u16,
    #[cfg(feature = "pio-real")]
    tx_timeout_count: u32,
    #[cfg(feature = "pio-real")]
    rx_timeout_count: u32,
    initialized: bool,
}

#[cfg(not(test))]
impl Rp2040FredTransport {
    pub const fn new() -> Self {
        Self {
            #[cfg(feature = "mock-bus")]
            status_fcf0: 0x00,
            #[cfg(feature = "mock-bus")]
            response_fcf1: 0x00,
            last_write_word: 0,
            last_read_word: 0,
            #[cfg(feature = "pio-real")]
            tx_timeout_count: 0,
            #[cfg(feature = "pio-real")]
            rx_timeout_count: 0,
            initialized: false,
        }
    }

    pub fn init(&mut self, p: Peripherals) {
        if self.initialized {
            return;
        }

        #[cfg(feature = "mock-bus")]
        {
            let _ = p;
        }

        #[cfg(feature = "pio-real")]
        {
            self.init_pio_real(p);
        }

        self.initialized = true;
    }

    pub fn write_fc80(&mut self, cmd: u8) {
        let word = compose_bus_word(reg::FC80, cmd);
        self.last_write_word = word;

        #[cfg(feature = "pio-real")]
        {
            if !pio0_tx_push(0, word as u32) {
                self.tx_timeout_count = self.tx_timeout_count.wrapping_add(1);
            }
        }
    }

    #[cfg(feature = "mock-bus")]
    pub fn inject_mock_reply(&mut self, reply: FredReply) {
        self.status_fcf0 = reply.status_fcf0;
        self.response_fcf1 = reply.response_fcf1;
    }

    pub fn read_fcf0(&mut self) -> u8 {
        let word = compose_bus_word(reg::FCF0, 0x00);
        self.last_read_word = word;

        #[cfg(feature = "pio-real")]
        {
            if !pio0_tx_push(1, word as u32) {
                self.tx_timeout_count = self.tx_timeout_count.wrapping_add(1);
                return 0xFF;
            }
            return match pio0_rx_pull(1) {
                Some(v) => v as u8,
                None => {
                    self.rx_timeout_count = self.rx_timeout_count.wrapping_add(1);
                    0xFF
                }
            };
        }

        #[cfg(feature = "mock-bus")]
        {
            self.status_fcf0
        }

        #[cfg(all(not(feature = "mock-bus"), not(feature = "pio-real")))]
        {
            0x00
        }
    }

    pub fn read_fcf1(&mut self) -> u8 {
        let word = compose_bus_word(reg::FCF1, 0x00);
        self.last_read_word = word;

        #[cfg(feature = "pio-real")]
        {
            if !pio0_tx_push(1, word as u32) {
                self.tx_timeout_count = self.tx_timeout_count.wrapping_add(1);
                return 0xFF;
            }
            return match pio0_rx_pull(1) {
                Some(v) => v as u8,
                None => {
                    self.rx_timeout_count = self.rx_timeout_count.wrapping_add(1);
                    0xFF
                }
            };
        }

        #[cfg(feature = "mock-bus")]
        {
            self.response_fcf1
        }

        #[cfg(all(not(feature = "mock-bus"), not(feature = "pio-real")))]
        {
            0x00
        }
    }

    #[cfg(feature = "pio-real")]
    fn init_pio_real(&mut self, p: Peripherals) {
        self.init_pio_peripherals(
            p.PIO0, p.PIN_0, p.PIN_1, p.PIN_2, p.PIN_3, p.PIN_4, p.PIN_5, p.PIN_6, p.PIN_7,
            p.PIN_8, p.PIN_9, p.PIN_10, p.PIN_11, p.PIN_12, p.PIN_13, p.PIN_14, p.PIN_15, p.PIN_16,
            p.PIN_17, p.PIN_20, p.PIN_27, p.PIN_28,
        );
    }

    #[cfg(feature = "pio-real")]
    pub fn init_pio_peripherals(
        &mut self,
        pio0: Peri<'static, PIO0>,
        pin_0: Peri<'static, PIN_0>,
        pin_1: Peri<'static, PIN_1>,
        pin_2: Peri<'static, PIN_2>,
        pin_3: Peri<'static, PIN_3>,
        pin_4: Peri<'static, PIN_4>,
        pin_5: Peri<'static, PIN_5>,
        pin_6: Peri<'static, PIN_6>,
        pin_7: Peri<'static, PIN_7>,
        pin_8: Peri<'static, PIN_8>,
        pin_9: Peri<'static, PIN_9>,
        pin_10: Peri<'static, PIN_10>,
        pin_11: Peri<'static, PIN_11>,
        pin_12: Peri<'static, PIN_12>,
        pin_13: Peri<'static, PIN_13>,
        pin_14: Peri<'static, PIN_14>,
        pin_15: Peri<'static, PIN_15>,
        pin_16: Peri<'static, PIN_16>,
        pin_17: Peri<'static, PIN_17>,
        pin_20: Peri<'static, PIN_20>,
        pin_27: Peri<'static, PIN_27>,
        pin_28: Peri<'static, PIN_28>,
    ) {
        if self.initialized {
            return;
        }

        let write_program = pio::pio_file!(
            "../pio/fred_transport.pio",
            select_program("fred_bus_write"),
            options(max_program_size = 32)
        );
        let read_program = pio::pio_file!(
            "../pio/fred_transport.pio",
            select_program("fred_bus_read"),
            options(max_program_size = 32)
        );

        let mut pio = Pio::new(pio0, Irqs);

        let loaded_write = pio.common.load_program(&write_program.program);
        let loaded_read = pio.common.load_program(&read_program.program);

        // Shared data bus D0..D7 and low address bus A0..A7.
        let d0 = pio.common.make_pio_pin(pin_0);
        let d1 = pio.common.make_pio_pin(pin_1);
        let d2 = pio.common.make_pio_pin(pin_2);
        let d3 = pio.common.make_pio_pin(pin_3);
        let d4 = pio.common.make_pio_pin(pin_4);
        let d5 = pio.common.make_pio_pin(pin_5);
        let d6 = pio.common.make_pio_pin(pin_6);
        let d7 = pio.common.make_pio_pin(pin_7);

        let a0 = pio.common.make_pio_pin(pin_8);
        let a1 = pio.common.make_pio_pin(pin_9);
        let a2 = pio.common.make_pio_pin(pin_10);
        let a3 = pio.common.make_pio_pin(pin_11);
        let a4 = pio.common.make_pio_pin(pin_12);
        let a5 = pio.common.make_pio_pin(pin_13);
        let a6 = pio.common.make_pio_pin(pin_14);
        let a7 = pio.common.make_pio_pin(pin_15);

        // Control lines (sideset order must match PIO source).
        let rnw = pio.common.make_pio_pin(pin_16);
        let mhz1e = pio.common.make_pio_pin(pin_17);
        let fred_n = pio.common.make_pio_pin(pin_20);
        let data_dir = pio.common.make_pio_pin(pin_27);
        let data_oe_n = pio.common.make_pio_pin(pin_28);

        let bus_out_pins = [
            &d0, &d1, &d2, &d3, &d4, &d5, &d6, &d7, &a0, &a1, &a2, &a3, &a4, &a5, &a6, &a7,
        ];
        let data_in_pins = [&d0, &d1, &d2, &d3, &d4, &d5, &d6, &d7];
        let side_pins = [&mhz1e, &rnw, &fred_n, &data_dir, &data_oe_n];

        let mut write_cfg = Config::default();
        write_cfg.use_program(&loaded_write, &side_pins);
        write_cfg.set_out_pins(&bus_out_pins);
        write_cfg.shift_out = ShiftConfig {
            threshold: 32,
            direction: ShiftDirection::Right,
            auto_fill: false,
        };
        write_cfg.clock_divider = 125u8.into();

        pio.sm0.set_config(&write_cfg);
        pio.sm0.set_pin_dirs(Direction::Out, &bus_out_pins);
        pio.sm0.set_pin_dirs(Direction::Out, &side_pins);
        pio.sm0.set_pins(Level::High, &side_pins);
        pio.sm0.clear_fifos();

        let mut read_cfg = Config::default();
        read_cfg.use_program(&loaded_read, &side_pins);
        read_cfg.set_out_pins(&bus_out_pins);
        read_cfg.set_in_pins(&data_in_pins);
        read_cfg.shift_in = ShiftConfig {
            threshold: 8,
            direction: ShiftDirection::Right,
            auto_fill: false,
        };
        read_cfg.shift_out = ShiftConfig {
            threshold: 32,
            direction: ShiftDirection::Right,
            auto_fill: false,
        };
        read_cfg.clock_divider = 125u8.into();

        pio.sm1.set_config(&read_cfg);
        pio.sm1.set_pin_dirs(Direction::Out, &bus_out_pins);
        pio.sm1.set_pin_dirs(Direction::Out, &side_pins);
        pio.sm1.set_pins(Level::High, &side_pins);
        pio.sm1.clear_fifos();

        pio.common.apply_sm_batch(|batch| {
            batch.restart(&mut pio.sm0);
            batch.restart(&mut pio.sm1);
            batch.set_enable(&mut pio.sm0, true);
            batch.set_enable(&mut pio.sm1, true);
        });

        // Keep PIO ownership for the life of the firmware so state machines remain configured.
        core::mem::forget(pio);
        self.initialized = true;
    }
}

#[cfg(feature = "pio-real")]
#[inline]
fn pio0_tx_push(sm: usize, value: u32) -> bool {
    let txfull_mask = 1u8 << sm;
    let mut spins = 0u32;
    while pac::PIO0.fstat().read().txfull() & txfull_mask != 0 {
        spins = spins.wrapping_add(1);
        if spins >= 100_000 {
            return false;
        }
    }
    pac::PIO0.txf(sm).write_value(value);
    true
}

#[cfg(feature = "pio-real")]
#[inline]
fn pio0_rx_pull(sm: usize) -> Option<u32> {
    let rxempty_mask = 1u8 << sm;
    let mut spins = 0u32;
    while pac::PIO0.fstat().read().rxempty() & rxempty_mask != 0 {
        spins = spins.wrapping_add(1);
        if spins >= 100_000 {
            return None;
        }
    }
    Some(pac::PIO0.rxf(sm).read())
}

#[inline]
const fn compose_bus_word(addr_lo: u8, data: u8) -> u16 {
    // PIO pin mapping:
    // - bits [7:0]   -> D0..D7
    // - bits [15:8]  -> A0..A7
    ((addr_lo as u16) << 8) | (data as u16)
}
