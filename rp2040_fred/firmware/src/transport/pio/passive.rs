use embassy_rp::{Peri, peripherals::PIO0, pio::{Common, Config, Direction, Instance, Pio, PioBatch, PioPin, ShiftConfig, ShiftDirection, StateMachine}, pio_programs::clock_divider::calculate_pio_clock_divider_value};

use crate::{PioIrqs, resources::PioResources};

pub struct PassivePio<
    'sm,
    PIO: Instance,
    const SR: usize,
> {
    _common: Common<'sm, PIO>,
    pub read: StateMachine<'sm, PIO, SR>,
}

pub type ThisPassivePio<'a> = PassivePio<'a, PIO0, 0>;

impl<'a> ThisPassivePio<'a> {

    pub fn setup(pio_resources: PioResources, debug_pin: Peri<'a, impl PioPin + 'a>) -> ThisPassivePio<'a> {

        let program = pio::pio_file!(
            "../pio/passive_sniffer.pio",
            select_program("fred_passive_sniffer"),
            options(max_program_size = 32)
        );

        let mut pio = Pio::new(pio_resources.pio0, PioIrqs);

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

        let pio_debug_pin = pio.common.make_pio_pin(debug_pin);

        let in_pins = [
            &p0, &p1, &p2, &p3, &p4, &p5, &p6, &p7, // data bus
            &p8, &p9, &p10, &p11, &p12, &p13, &p14, &p15, // addr bus
            &p16, // 1MHzE
            &p17, // RnW
            &p18, // FRED
        ];

        let mut cfg = Config::default();
        cfg.use_program(&loaded, &[&pio_debug_pin]);
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
        pio.sm0.set_pin_dirs(Direction::Out, &[&pio_debug_pin]);
        pio.sm0.clear_fifos();

        let mut batch = PioBatch::new();
        batch.restart(&mut pio.sm0);
        batch.set_enable(&mut pio.sm0, true);
        batch.execute();

        let _ = pio.sm0.rx().stalled();

        ThisPassivePio {
            read: pio.sm0,
            _common: pio.common,
        }

    }

}