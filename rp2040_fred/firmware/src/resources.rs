use assign_resources::assign_resources;
use embassy_rp::{peripherals, Peri};

assign_resources! {
    core1: Core1Resources {
        core1: CORE1,
    }
    usb: UsbResources {
        usb: USB,
    }
    pio: PioResources {
        pio0: PIO0,
        pio1: PIO1,
        pin_0: PIN_0,
        pin_1: PIN_1,
        pin_2: PIN_2,
        pin_3: PIN_3,
        pin_4: PIN_4,
        pin_5: PIN_5,
        pin_6: PIN_6,
        pin_7: PIN_7,
        pin_8: PIN_8,
        pin_9: PIN_9,
        pin_10: PIN_10,
        pin_11: PIN_11,
        pin_12: PIN_12,
        pin_13: PIN_13,
        pin_14: PIN_14,
        pin_15: PIN_15,
        pin_16: PIN_16,
        pin_17: PIN_17,
        pin_18: PIN_18,
    }
    dir: DirectionResources {
        pin_19: PIN_19,
        pin_20: PIN_20,
        pin_21: PIN_21,
    }
    debug27: DebugPin27Resources {
        pin: PIN_27,
    }
    debug28: DebugPin28Resources {
        pin: PIN_28,
    }
    main: MainResources {
        led: PIN_25,
    }
}
