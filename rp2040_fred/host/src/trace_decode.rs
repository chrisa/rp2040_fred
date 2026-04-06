use std::fmt;
use std::io;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisSnapshot {
    pub negative: bool,
    pub value: u32,
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
    pub fn x_digits(self) -> String {
        format_axis(self.x)
    }

    pub fn z_digits(self) -> String {
        format_axis(self.z)
    }
}

impl fmt::Display for FeedbackSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:08}  {}  {}  {:04}  {:04}",
            self.sample_index,
            self.x_digits(),
            self.z_digits(),
            self.rpm_raw,
            self.rpm_display
        )
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

pub struct FeedbackDecoder {
    pending_cmd: Option<u8>,
    x: AxisState,
    z: AxisState,
    rpm_pairs: [u8; 2],
    rpm_mask: u8,
    last_emitted: Option<FeedbackSnapshot>,
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
        let cycle = TraceCycle::from_sample(sample)?;
        self.ingest_cycle(sample_index, cycle)
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

        let snapshot = self.snapshot(sample_index)?;
        if self.last_emitted == Some(snapshot) {
            return None;
        }
        self.last_emitted = Some(snapshot);
        Some(snapshot)
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

pub fn parse_trace_line(line: &str) -> io::Result<Option<(u64, u32)>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("step") {
        return Ok(None);
    }

    let mut parts = trimmed.split_whitespace();
    let step = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing step field"))?;
    let sample = parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing sample field"))?;

    let step = step.parse::<u64>().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid step value {step:?}: {e}"),
        )
    })?;
    let sample = sample
        .strip_prefix("0x")
        .or_else(|| sample.strip_prefix("0X"))
        .unwrap_or(sample);
    let sample = u32::from_str_radix(sample, 16).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid sample value {sample:?}: {e}"),
        )
    })?;

    Ok(Some((step, sample)))
}

fn is_packed_bcd(byte: u8) -> bool {
    (byte >> 4) <= 9 && (byte & 0x0F) <= 9
}

fn bcd_pair_value(byte: u8) -> u32 {
    ((byte >> 4) as u32) * 10 + (byte & 0x0F) as u32
}

fn format_axis(axis: AxisSnapshot) -> String {
    format!("{}{:06}", if axis.negative { "-" } else { "+" }, axis.value)
}

#[cfg(test)]
mod tests {
    use super::{parse_trace_line, FeedbackDecoder, TraceCycle};

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
    fn parser_skips_header_and_reads_hex_sample() {
        assert!(parse_trace_line("step  sample      D    A   RnW CLK FREDn")
            .expect("header")
            .is_none());
        let (step, sample_word) = parse_trace_line("0718  0x0003F101  01  F1   R   1    0")
            .expect("line")
            .expect("sample");
        assert_eq!(step, 718);
        assert_eq!(sample_word, 0x0003F101);
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
}
