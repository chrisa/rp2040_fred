use embassy_rp::Peri;
use embassy_rp::gpio::Level;
use embassy_rp::peripherals::{PIN_19, PIO1};
use embassy_rp::pio::{
    Common, Config, Direction, Instance, Pio, PioBatch, PioPin, ShiftConfig, ShiftDirection, StateMachine
};
use embassy_rp::pio_programs::clock_divider::calculate_pio_clock_divider_value;
use rp2040_fred_firmware::log_info;
use rp_pac as pac;

use crate::resources::PioResources;
use crate::PioIrqs;

pub struct MasterPio<
    'sm,
    PIO: Instance,
    const SCL: usize,
    const SC: usize,
    const SW: usize,
    const SR: usize,
> {
    _common: Common<'sm, PIO>,
    _clock: StateMachine<'sm, PIO, SCL>,
    pub control: StateMachine<'sm, PIO, SC>,
    pub write: StateMachine<'sm, PIO, SW>,
    pub read: StateMachine<'sm, PIO, SR>,
}

// with concrete SM assignments
pub type ThisMasterPio<'a> = MasterPio<'a, PIO1, 0, 1, 2, 3>;

impl<'a> ThisMasterPio<'a> {
    pub fn setup(pio_resources: PioResources, pin_19: Peri<'a, PIN_19>, debug_pin: Peri<'a, impl PioPin + 'a>) -> ThisMasterPio<'a> {

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

        let mut pio = Pio::new(pio_resources.pio1, PioIrqs);

        let mut clock = pio.sm0;
        let mut control = pio.sm1;
        let mut write = pio.sm2;
        let mut read = pio.sm3;

        let clock_program = pio.common.load_program(&fred_bm_clock.program);
        let control_program = pio.common.load_program(&fred_bm_control.program);
        let write_program = pio.common.load_program(&fred_bm_data_write.program);
        let read_program = pio.common.load_program(&fred_bm_data_read.program);

        log_info!("PIO programs loaded");

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
        let p19 = pio.common.make_pio_pin(pin_19);
        let pio_debug_pin = pio.common.make_pio_pin(debug_pin);

        let data_bus_pins = [&p0, &p1, &p2, &p3, &p4, &p5, &p6, &p7];

        let addr_bus_pins = [&p8, &p9, &p10, &p11, &p12, &p13, &p14, &p15];

        let control_pins = [
            &p17, // RnW
            &p18, // FRED
        ];

        let clock_pins = [
            &p16, // 1MHzE
        ];

        let data_dir_pin = [&p19];

        let read_debug_pin = [&pio_debug_pin];

        clock.set_pin_dirs(Direction::In, &data_bus_pins);
        clock.set_pin_dirs(Direction::Out, &addr_bus_pins);
        clock.set_pin_dirs(Direction::Out, &data_dir_pin);
        clock.set_pin_dirs(Direction::Out, &read_debug_pin);

        clock.set_pins(Level::Low, &control_pins);
        clock.set_pin_dirs(Direction::Out, &control_pins);
        clock.set_pins(Level::Low, &clock_pins);
        clock.set_pin_dirs(Direction::Out, &clock_pins);

        clock.clear_fifos();
        control.clear_fifos();
        write.clear_fifos();
        read.clear_fifos();

        let mut clock_cfg = Config::default();
        clock_cfg.use_program(&clock_program, &clock_pins);
        clock_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 16_000_000);
        clock.set_config(&clock_cfg);

        let mut control_cfg = Config::default();
        control_cfg.use_program(&control_program, &control_pins);
        control_cfg.set_out_pins(&addr_bus_pins);
        control_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 40_000_000);
        control_cfg.shift_out = ShiftConfig {
            threshold: 32,
            direction: ShiftDirection::Left,
            auto_fill: false,
        };
        control.set_config(&control_cfg);

        let mut write_cfg = Config::default();
        write_cfg.use_program(&write_program, &data_dir_pin);
        write_cfg.set_out_pins(&data_bus_pins);
        write_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 20_000_000);
        write_cfg.shift_out = ShiftConfig {
            threshold: 8,
            direction: ShiftDirection::Left,
            auto_fill: false,
        };
        write.set_config(&write_cfg);

        let mut read_cfg = Config::default();
        read_cfg.use_program(&read_program, &read_debug_pin);
        read_cfg.set_in_pins(&data_bus_pins);
        read_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 125_000_000);
        read_cfg.shift_in = ShiftConfig {
            threshold: 8,
            direction: ShiftDirection::Left,
            auto_fill: false,
        };
        read.set_config(&read_cfg);

        let mut batch = PioBatch::new();
        batch.restart(&mut clock);
        batch.set_enable(&mut clock, true);
        batch.restart(&mut control);
        batch.set_enable(&mut control, true);
        batch.restart(&mut write);
        batch.set_enable(&mut write, true);
        batch.restart(&mut read);
        batch.set_enable(&mut read, true);
        batch.execute();

        [16, 17, 18, 19, 20].iter().for_each(|n| {
            pac::PADS_BANK0.gpio(*n).modify(|w| {
                w.set_pue(true); // pull-up enable
                w.set_pde(false); // pull-down disable
            });
        });

        log_info!("PIO initialised on core1");

        ThisMasterPio {
            _clock: clock,
            control,
            write,
            read,
            _common: pio.common,
        }
    }
}
