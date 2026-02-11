#![allow(dead_code)]

// Filled from `rp2040_fred/pinmap_template.md`.
pub mod gpio {
    // Shared data bus D0..D7
    pub const D0: u8 = 0;
    pub const D1: u8 = 1;
    pub const D2: u8 = 2;
    pub const D3: u8 = 3;
    pub const D4: u8 = 4;
    pub const D5: u8 = 5;
    pub const D6: u8 = 6;
    pub const D7: u8 = 7;

    // Address bus A0..A7
    pub const A0: u8 = 8;
    pub const A1: u8 = 9;
    pub const A2: u8 = 10;
    pub const A3: u8 = 11;
    pub const A4: u8 = 12;
    pub const A5: u8 = 13;
    pub const A6: u8 = 14;
    pub const A7: u8 = 15;

    // Control outputs
    pub const RNW: u8 = 16;
    pub const MHZ1E: u8 = 17;
    pub const FRED_N: u8 = 20;
    pub const DATA_DIR: u8 = 27;
    pub const DATA_OE_N: u8 = 28;
}

pub mod reg {
    pub const FC80: u8 = 0x80;
    pub const FCF0: u8 = 0xF0;
    pub const FCF1: u8 = 0xF1;
}
