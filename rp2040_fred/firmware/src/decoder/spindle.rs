use crate::decoder::{bcd_pair_value, is_packed_bcd};

const RPM_SETTLE_MIN_SAMPLES: u8 = 4;
const RPM_SETTLE_STABLE_SAMPLES: u8 = 2;
const RPM_SETTLE_MAX_SAMPLES: u8 = 12;
const RPM_SETTLE_TOLERANCE: u16 = 20;

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
    fn from_raw(rpm_raw: u16) -> Self {
        Self {
            rpm_raw,
            rpm_display: (rpm_raw / 10) * 10,
        }
    }

    pub fn rpm(&self) -> u16 {
        self.rpm_display
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpindleState {
    rpm_pairs: [u8; 2],
    rpm_mask: u8,
    accepted: SpindleSnapshot,
    accepted_valid: bool,
    settling: bool,
    skip_current_snapshot: bool,
    settle_samples: u8,
    stable_samples: u8,
    last_raw: u16,
}

impl Default for SpindleState {
    fn default() -> Self {
        Self {
            rpm_pairs: [0; 2],
            rpm_mask: 0,
            accepted: SpindleSnapshot::default(),
            accepted_valid: false,
            settling: false,
            skip_current_snapshot: false,
            settle_samples: 0,
            stable_samples: 0,
            last_raw: 0,
        }
    }
}

impl SpindleState {
    pub fn reset(&mut self) {
        self.rpm_mask = 0;
    }

    pub fn trigger(&mut self) {
        self.settling = true;
        self.skip_current_snapshot = true;
        self.settle_samples = 0;
        self.stable_samples = 0;
        self.last_raw = 0;
        self.rpm_mask = 0;
    }

    pub fn set_pair(&mut self, idx: usize, response: u8) {
        if is_packed_bcd(response) {
            self.rpm_pairs[idx] = response;
            self.rpm_mask |= 1 << idx;
        } else {
            self.rpm_mask &= !(1 << idx);
        }
    }

    pub fn snapshot(&mut self) -> Option<SpindleSnapshot> {
        if self.skip_current_snapshot {
            self.skip_current_snapshot = false;
            return None;
        }
        if self.rpm_mask != 0b11 {
            return None;
        }

        let rpm_raw =
            (bcd_pair_value(self.rpm_pairs[0]) * 100 + bcd_pair_value(self.rpm_pairs[1])) as u16;
        let snapshot = SpindleSnapshot::from_raw(rpm_raw);

        if !self.accepted_valid || self.settling {
            return self.observe_settling_sample(snapshot);
        }

        if rpm_close(rpm_raw, self.accepted.rpm_raw) {
            self.accepted = snapshot;
            return Some(snapshot);
        }

        self.begin_settling(rpm_raw);
        None
    }

    fn begin_settling(&mut self, rpm_raw: u16) {
        self.settling = true;
        self.settle_samples = 1;
        self.stable_samples = 1;
        self.last_raw = rpm_raw;
    }

    fn observe_settling_sample(&mut self, snapshot: SpindleSnapshot) -> Option<SpindleSnapshot> {
        if self.settle_samples == 0 {
            self.begin_settling(snapshot.rpm_raw);
            return None;
        }

        if rpm_close(snapshot.rpm_raw, self.last_raw) {
            self.stable_samples = self.stable_samples.saturating_add(1);
        } else {
            self.stable_samples = 1;
        }

        self.settle_samples = self.settle_samples.saturating_add(1);
        self.last_raw = snapshot.rpm_raw;

        if (self.settle_samples >= RPM_SETTLE_MIN_SAMPLES
            && self.stable_samples >= RPM_SETTLE_STABLE_SAMPLES)
            || self.settle_samples >= RPM_SETTLE_MAX_SAMPLES
        {
            self.accepted = snapshot;
            self.accepted_valid = true;
            self.settling = false;
            return Some(snapshot);
        }

        None
    }
}

fn rpm_close(a: u16, b: u16) -> bool {
    a.abs_diff(b) <= RPM_SETTLE_TOLERANCE
}
