#![allow(dead_code)]

#[derive(Clone, Copy, Debug, Default)]
pub struct DroSnapshot {
    pub x_counts: i32,
    pub z_counts: i32,
    pub rpm: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Calibration {
    pub x_counts_per_mm: f32,
    pub z_counts_per_mm: f32,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            x_counts_per_mm: 100.0,
            z_counts_per_mm: 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct AxisScratch {
    sign_neg: bool,
    b2: u8,
    b1: u8,
    b0: u8,
}

impl Default for AxisScratch {
    fn default() -> Self {
        Self {
            sign_neg: false,
            b2: 0,
            b1: 0,
            b0: 0,
        }
    }
}

impl AxisScratch {
    fn counts(&self) -> i32 {
        let mag = ((self.b2 as u32) << 16) | ((self.b1 as u32) << 8) | self.b0 as u32;
        if self.sign_neg {
            -(mag as i32)
        } else {
            mag as i32
        }
    }
}

pub struct DroAssembler {
    x: AxisScratch,
    z: AxisScratch,
    rpm_hi: u8,
    rpm_lo: u8,
}

impl DroAssembler {
    pub const fn new() -> Self {
        Self {
            x: AxisScratch {
                sign_neg: false,
                b2: 0,
                b1: 0,
                b0: 0,
            },
            z: AxisScratch {
                sign_neg: false,
                b2: 0,
                b1: 0,
                b0: 0,
            },
            rpm_hi: 0,
            rpm_lo: 0,
        }
    }

    pub fn on_fc80_fcf1(&mut self, cmd: u8, response: u8) {
        match cmd {
            0x03 => self.x.sign_neg = response != 0,
            0x02 => self.x.b2 = response,
            0x01 => self.x.b1 = response,
            0x00 => self.x.b0 = response,
            0x07 => self.z.sign_neg = response != 0,
            0x06 => self.z.b2 = response,
            0x05 => self.z.b1 = response,
            0x04 => self.z.b0 = response,
            0x0D => self.rpm_hi = response,
            0x0C => self.rpm_lo = response,
            _ => {}
        }
    }

    pub fn snapshot(&self) -> DroSnapshot {
        DroSnapshot {
            x_counts: self.x.counts(),
            z_counts: self.z.counts(),
            rpm: ((self.rpm_hi as u16) << 8) | self.rpm_lo as u16,
        }
    }
}

pub fn counts_to_mm(snapshot: DroSnapshot, cal: Calibration) -> (f32, f32, u16) {
    // CNCMAN uses diameter semantics for X (x*2), direct for Z.
    let x_mm = ((snapshot.x_counts as f32) * 2.0) / cal.x_counts_per_mm;
    let z_mm = (snapshot.z_counts as f32) / cal.z_counts_per_mm;
    (x_mm, z_mm, snapshot.rpm)
}

#[cfg(test)]
mod tests {
    use super::{counts_to_mm, Calibration, DroAssembler};

    #[test]
    fn assembler_rebuilds_values() {
        let mut a = DroAssembler::new();
        a.on_fc80_fcf1(0x03, 0x01); // negative
        a.on_fc80_fcf1(0x02, 0x00);
        a.on_fc80_fcf1(0x01, 0x00);
        a.on_fc80_fcf1(0x00, 0x64); // 100

        a.on_fc80_fcf1(0x07, 0x00); // positive
        a.on_fc80_fcf1(0x06, 0x00);
        a.on_fc80_fcf1(0x05, 0x00);
        a.on_fc80_fcf1(0x04, 0xC8); // 200

        a.on_fc80_fcf1(0x0D, 0x07);
        a.on_fc80_fcf1(0x0C, 0xD0); // 2000 rpm

        let s = a.snapshot();
        assert_eq!(s.x_counts, -100);
        assert_eq!(s.z_counts, 200);
        assert_eq!(s.rpm, 2000);

        let (x_mm, z_mm, rpm) = counts_to_mm(
            s,
            Calibration {
                x_counts_per_mm: 100.0,
                z_counts_per_mm: 100.0,
            },
        );
        assert!((x_mm + 2.0).abs() < 0.0001);
        assert!((z_mm - 2.0).abs() < 0.0001);
        assert_eq!(rpm, 2000);
    }
}
