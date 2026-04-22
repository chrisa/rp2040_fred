use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{
    Common, Config, Direction, Instance, InterruptHandler, Pio, PioBatch, ShiftConfig,
    ShiftDirection, StateMachine,
};
use embassy_rp::pio_programs::clock_divider::calculate_pio_clock_divider_value;
use rp_pac as pac;

use crate::resources::PioResources;

macro_rules! log_info {
    ($($arg:tt)*) => {
        defmt::info!($($arg)*);
    };
}

bind_interrupts!(struct Pio0Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});
// bind_interrupts!(struct Pio1Irqs {
//     PIO1_IRQ_0 => InterruptHandler<PIO1>;
// });

pub struct Bus<
    'sm,
    PIO: Instance,
    const SCL: usize,
    const SC: usize,
    const SW: usize,
    const SR: usize,
> {
    _common: Common<'sm, PIO>,
    _dir_a: Output<'sm>,
    _clock: StateMachine<'sm, PIO, SCL>,
    control: StateMachine<'sm, PIO, SC>,
    write: StateMachine<'sm, PIO, SW>,
    read: StateMachine<'sm, PIO, SR>,
}

// with concrete SM assignments
pub type ThisBus<'a> = Bus<'a, PIO0, 0, 1, 2, 3>;

impl<'a> ThisBus<'a> {
    pub async fn command_cycle(&mut self, cmd: u8) -> u8 {
        // 1. Poll `F0` until bit 0 clears.
        // 2. Write one command byte to `80`.
        // 3. Poll `F0` again until bit 0 clears.
        // 4. Read one response byte from `F1`.
        self.poll_until(0xF0, 0x01).await;
        self.write_cycle(0x80, cmd).await;
        self.poll_until(0xF0, 0x01).await;
        self.read_cycle(0xF1).await
    }

    pub async fn poll_until(&mut self, addr: u8, mask: u8) {
        let addr_payload = 0x0001_0000u32 | ((addr as u32) << 24);
        self.read.clear_fifos();
        loop {
            self.control.tx().wait_push(addr_payload).await;
            if let Some(r) = self.read.rx().try_pull() {
                if (r >> 24) as u8 & mask == 0 {
                    break;
                }
            }
        }
    }

    pub async fn write_cycle(&mut self, addr: u8, data: u8) {
        let data_payload = 0xFF00_0000u32 | ((data as u32) << 16);
        // let addr_payload = 0x0000_0000u32 | ((addr as u32) << 24);
        let addr_payload = (addr as u32) << 24;
        self.write.tx().wait_push(data_payload).await;
        self.control.tx().wait_push(addr_payload).await;
    }

    pub async fn read_cycle(&mut self, addr: u8) -> u8 {
        let addr_payload = 0x0001_0000u32 | ((addr as u32) << 24);
        self.control.tx().wait_push(addr_payload).await;
        self.read.rx().wait_pull().await as u8
        // self.read.rx().pull() as u8
    }

    pub fn setup(pio_resources: PioResources) -> ThisBus<'a> {
        // address data-dir high for output.
        let mut dir_a = Output::new(pio_resources.pin_26, Level::High);
        dir_a.set_high();

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

        let mut pio0 = Pio::new(pio_resources.pio0, Pio0Irqs);

        let mut clock = pio0.sm0;
        let mut control = pio0.sm1;
        let mut write = pio0.sm2;
        let mut read = pio0.sm3;

        let clock_program = pio0.common.load_program(&fred_bm_clock.program);
        let control_program = pio0.common.load_program(&fred_bm_control.program);
        let write_program = pio0.common.load_program(&fred_bm_data_write.program);
        let read_program = pio0.common.load_program(&fred_bm_data_read.program);

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
        let p27 = pio0.common.make_pio_pin(pio_resources.pin_27);

        let data_bus_pins = [&p0, &p1, &p2, &p3, &p4, &p5, &p6, &p7];

        let addr_bus_pins = [&p8, &p9, &p10, &p11, &p12, &p13, &p14, &p15];

        let control_pins = [
            &p17, // RnW
            &p18, // NMI
            &p19, // IRQ
            &p20, // FRED
        ];

        let clock_pins = [
            &p16, // 1MHzE
        ];

        let data_dir_pin = [&p27];

        clock.set_pin_dirs(Direction::In, &data_bus_pins);
        clock.set_pin_dirs(Direction::Out, &addr_bus_pins);
        clock.set_pin_dirs(Direction::Out, &data_dir_pin);

        // setting up control pins for open-drain output
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
        read_cfg.use_program(&read_program, &[]);
        read_cfg.set_in_pins(&data_bus_pins);
        read_cfg.clock_divider = calculate_pio_clock_divider_value(125_000_000, 50_000_000);
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

        ThisBus {
            _clock: clock,
            control,
            write,
            read,
            _dir_a: dir_a,
            _common: pio0.common,
        }
    }
}
