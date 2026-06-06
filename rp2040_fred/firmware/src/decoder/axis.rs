use crate::decoder::{bcd_pair_value, is_packed_bcd};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AxisSnapshot {
    pub negative: bool,
    pub value: u32,
}

impl Default for AxisSnapshot {
    fn default() -> Self {
        Self {
            negative: false,
            value: 0,
        }
    }
}

impl AxisSnapshot {
    pub fn count(&self) -> i32 {
        if self.negative {
            -(self.value as i32)
        } else {
            self.value as i32
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AxisState {
    sign_seen: bool,
    negative: bool,
    pairs: [u8; 3],
    pair_mask: u8,
}

impl Default for AxisState {
    fn default() -> Self {
        Self {
            sign_seen: false,
            negative: false,
            pairs: [0; 3],
            pair_mask: 0,
        }
    }
}

impl AxisState {
    pub fn reset(&mut self) {
        self.pair_mask = 0;
        self.sign_seen = false;
    }

    pub fn set_sign(&mut self, response: u8) {
        self.sign_seen = true;
        self.negative = response != 0;
    }

    pub fn set_pair(&mut self, idx: usize, response: u8) {
        if is_packed_bcd(response) {
            self.pairs[idx] = response;
            self.pair_mask |= 1 << idx;
        } else {
            self.pair_mask &= !(1 << idx);
        }
    }

    pub fn snapshot(&self) -> Option<AxisSnapshot> {
        if !self.sign_seen || self.pair_mask != 0b111 {
            return None;
        }

        Some(AxisSnapshot {
            negative: self.negative,
            value: bcd_pair_value(self.pairs[0]) * 10_000
                + bcd_pair_value(self.pairs[1]) * 100
                + bcd_pair_value(self.pairs[2]),
        })
    }
}
