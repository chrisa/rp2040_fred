use crate::decoder::{bcd_pair_value, is_packed_bcd};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpindleSnapshot {
    rpm_raw: u16,
    rpm_display: u16,
}

impl Default for SpindleSnapshot {
    fn default() -> Self {
        Self {
            rpm_raw: 0,
            rpm_display: 0,
        }
    }
}

impl SpindleSnapshot {
    pub fn rpm(&self) -> u16 {
        self.rpm_display
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpindleState {
    rpm_pairs: [u8; 2],
    rpm_mask: u8,
    count: usize,
}

impl Default for SpindleState {
    fn default() -> Self {
        Self {
            rpm_pairs: [0; 2],
            rpm_mask: 0,
            count: 0,
        }
    }
}

impl SpindleState {
    pub fn reset(&mut self) {
        self.rpm_mask = 0;
    }

    pub fn trigger(&mut self) {
        self.count = 0;
    }

    pub fn set_pair(&mut self, idx: usize, response: u8) {
        if is_packed_bcd(response) {
            self.rpm_pairs[idx] = response;
            self.rpm_mask |= 1 << idx;
            self.count += 1;
        } else {
            self.rpm_pairs[idx] = 0;
            self.rpm_mask |= 1 << idx;
        }
    }

    pub fn snapshot(&self) -> Option<SpindleSnapshot> {
        if self.rpm_mask != 0b11 {
            return None;
        }

        // only return a spindle snapshot after ten values
        if self.count < 10 {
            return None;
        }

        let rpm_raw =
            (bcd_pair_value(self.rpm_pairs[0]) * 100 + bcd_pair_value(self.rpm_pairs[1])) as u16;

        Some(SpindleSnapshot {
            rpm_raw,
            rpm_display: (rpm_raw / 10) * 10,
        })
    }
}
