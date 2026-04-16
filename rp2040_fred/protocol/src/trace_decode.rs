#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceCycle {
    pub data: u8,
    pub addr: u8,
    pub read: bool,
}

impl TraceCycle {
    pub fn from_sample(sample: u32) -> Option<Self> {
        let clock_high = ((sample >> 17) & 1) != 0;
        let fred_selected = ((sample >> 20) & 1) == 0;

        if !clock_high || !fred_selected {
            return None;
        }

        Some(Self {
            data: (sample & 0xFF) as u8,
            addr: ((sample >> 8) & 0xFF) as u8,
            read: ((sample >> 16) & 1) != 0,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisSnapshot {
    pub negative: bool,
    pub value: u32,
}

impl AxisSnapshot {
    pub fn count(&self) -> i32 {
        if self.negative {
            -(self.value as i32)
        } else {
            self.value as i32
        }
    }

    pub fn digits(&self) -> AxisDigits {
        AxisDigits::from_axis(self.negative, self.value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisDigits {
    bytes: [u8; 7],
}

impl AxisDigits {
    fn from_axis(negative: bool, value: u32) -> Self {
        let mut bytes = [0u8; 7];
        bytes[0] = if negative { b'-' } else { b'+' };

        let mut value = value;
        let mut idx = 6;
        while idx > 0 {
            bytes[idx] = b'0' + (value % 10) as u8;
            value /= 10;
            idx -= 1;
        }

        Self { bytes }
    }
}

impl core::fmt::Display for AxisDigits {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = core::str::from_utf8(&self.bytes).map_err(|_| core::fmt::Error)?;
        f.write_str(s)
    }
}

impl PartialEq<&str> for AxisDigits {
    fn eq(&self, other: &&str) -> bool {
        core::str::from_utf8(&self.bytes)
            .map(|s| s == *other)
            .unwrap_or(false)
    }
}

impl PartialEq<AxisDigits> for &str {
    fn eq(&self, other: &AxisDigits) -> bool {
        other == self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedbackSnapshot {
    pub sample_index: u64,
    pub x: AxisSnapshot,
    pub z: AxisSnapshot,
    pub rpm_raw: u16,
    pub rpm_display: u16,
}

impl FeedbackSnapshot {
    pub fn x_digits(&self) -> AxisDigits {
        self.x.digits()
    }

    pub fn z_digits(&self) -> AxisDigits {
        self.z.digits()
    }

    pub fn dro_snapshot(&self) -> DroSnapshot {
        DroSnapshot {
            x_counts: self.x.count(),
            z_counts: self.z.count(),
            rpm: self.rpm_raw,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AxisState {
    sign_seen: bool,
    negative: bool,
    pairs: [u8; 3],
    pair_mask: u8,
}

impl AxisState {
    fn set_sign(&mut self, response: u8) {
        self.sign_seen = true;
        self.negative = response != 0;
    }

    fn set_pair(&mut self, idx: usize, response: u8) {
        if !is_packed_bcd(response) {
            return;
        }
        self.pairs[idx] = response;
        self.pair_mask |= 1 << idx;
    }

    fn snapshot(&self) -> Option<AxisSnapshot> {
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

pub fn counts_to_mm(snapshot: DroSnapshot, cal: Calibration) -> (f32, f32, u16) {
    // CNCMAN uses diameter semantics for X (x*2), direct for Z.
    let x_mm = ((snapshot.x_counts as f32) * 2.0) / cal.x_counts_per_mm;
    let z_mm = (snapshot.z_counts as f32) / cal.z_counts_per_mm;
    (x_mm, z_mm, snapshot.rpm)
}

pub struct FeedbackDecoder {
    pending_cmd: Option<u8>,
    x: AxisState,
    z: AxisState,
    rpm_pairs: [u8; 2],
    rpm_mask: u8,
    last_emitted: Option<FeedbackSnapshot>,
}

impl Default for FeedbackDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackDecoder {
    pub const fn new() -> Self {
        Self {
            pending_cmd: None,
            x: AxisState {
                sign_seen: false,
                negative: false,
                pairs: [0; 3],
                pair_mask: 0,
            },
            z: AxisState {
                sign_seen: false,
                negative: false,
                pairs: [0; 3],
                pair_mask: 0,
            },
            rpm_pairs: [0; 2],
            rpm_mask: 0,
            last_emitted: None,
        }
    }

    pub fn ingest_sample(&mut self, sample_index: u64, sample: u32) -> Option<FeedbackSnapshot> {
        if let Some(cycle) = TraceCycle::from_sample(sample) {
            return self.ingest_cycle(sample_index, cycle);
        }
        None
    }

    pub fn ingest_cycle(
        &mut self,
        sample_index: u64,
        cycle: TraceCycle,
    ) -> Option<FeedbackSnapshot> {
        if cycle.addr == 0x80 && !cycle.read {
            self.pending_cmd = Some(cycle.data);
            return None;
        }

        if cycle.addr != 0xF1 || !cycle.read {
            return None;
        }

        let cmd = self.pending_cmd.take()?;
        self.apply_response(cmd, cycle.data);

        if cmd != 0x0C {
            return None;
        }

        if let Some(snapshot) = self.snapshot(sample_index) {
            if self.last_emitted == Some(snapshot) {
                return None;
            }
            self.last_emitted = Some(snapshot);
            return Some(snapshot)
        }

        None
    }

    fn apply_response(&mut self, cmd: u8, response: u8) {
        match cmd {
            0x03 => self.x.set_sign(response),
            0x02 => self.x.set_pair(0, response),
            0x01 => self.x.set_pair(1, response),
            0x00 => self.x.set_pair(2, response),
            0x07 => self.z.set_sign(response),
            0x06 => self.z.set_pair(0, response),
            0x05 => self.z.set_pair(1, response),
            0x04 => self.z.set_pair(2, response),
            0x0D => {
                if is_packed_bcd(response) {
                    self.rpm_pairs[0] = response;
                    self.rpm_mask |= 1 << 0;
                }
            }
            0x0C => {
                if is_packed_bcd(response) {
                    self.rpm_pairs[1] = response;
                    self.rpm_mask |= 1 << 1;
                }
            }
            _ => {}
        }
    }

    fn snapshot(&self, sample_index: u64) -> Option<FeedbackSnapshot> {
        if self.rpm_mask != 0b11 {
            return None;
        }

        let x = self.x.snapshot()?;
        let z = self.z.snapshot()?;
        let rpm_raw =
            (bcd_pair_value(self.rpm_pairs[0]) * 100 + bcd_pair_value(self.rpm_pairs[1])) as u16;

        Some(FeedbackSnapshot {
            sample_index,
            x,
            z,
            rpm_raw,
            rpm_display: (rpm_raw / 10) * 10,
        })
    }
}

fn is_packed_bcd(byte: u8) -> bool {
    (byte >> 4) <= 9 && (byte & 0x0F) <= 9
}

fn bcd_pair_value(byte: u8) -> u32 {
    ((byte >> 4) as u32) * 10 + (byte & 0x0F) as u32
}

#[cfg(test)]
mod tests {
    use super::{counts_to_mm, Calibration, DroSnapshot, FeedbackDecoder, TraceCycle};

    fn sample(data: u8, addr: u8, read: bool, clock_high: bool) -> u32 {
        (data as u32) | ((addr as u32) << 8) | ((read as u32) << 16) | ((clock_high as u32) << 17)
    }

    #[test]
    fn trace_cycle_requires_completed_fred_phase() {
        assert!(TraceCycle::from_sample(sample(0x12, 0x80, false, false)).is_none());
        let cycle = TraceCycle::from_sample(sample(0x12, 0x80, false, true)).expect("cycle");
        assert_eq!(cycle.data, 0x12);
        assert_eq!(cycle.addr, 0x80);
        assert!(!cycle.read);
    }

    #[test]
    fn decoder_builds_signed_axes_and_rounded_rpm() {
        let mut decoder = FeedbackDecoder::new();
        let seq = [
            (0x03, 0x01),
            (0x02, 0x00),
            (0x01, 0x06),
            (0x00, 0x52),
            (0x07, 0x00),
            (0x06, 0x00),
            (0x05, 0x12),
            (0x04, 0x34),
            (0x0D, 0x07),
            (0x0C, 0x83),
        ];

        let mut emitted = None;
        for (i, (cmd, response)) in seq.into_iter().enumerate() {
            let _ = decoder.ingest_sample(i as u64 * 2, sample(cmd, 0x80, false, true));
            emitted = decoder.ingest_sample(i as u64 * 2 + 1, sample(response, 0xF1, true, true));
        }

        let snapshot = emitted.expect("snapshot");
        assert!(snapshot.x.negative);
        assert_eq!(snapshot.x.value, 652);
        assert!(!snapshot.z.negative);
        assert_eq!(snapshot.z.value, 1234);
        assert_eq!(snapshot.rpm_raw, 783);
        assert_eq!(snapshot.rpm_display, 780);
        assert_eq!(snapshot.x_digits(), "-000652");
        assert_eq!(snapshot.z_digits(), "+001234");
    }

    #[test]
    fn counts_convert_to_mm() {
        let s = DroSnapshot {
            x_counts: -100,
            z_counts: 200,
            rpm: 2000,
        };
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
