use crate::resources::{FRED_PIN, ONE_MHZ_PIN, READ_WRITE_PIN};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceCycle {
    pub data: u8,
    pub addr: u8,
    pub read: bool,
}

impl TraceCycle {
    pub fn from_sample(sample: u32) -> Option<Self> {
        let clock_high = ((sample >> ONE_MHZ_PIN) & 1) != 0;
        let fred_selected = ((sample >> FRED_PIN) & 1) == 0;

        if !clock_high || !fred_selected {
            return None;
        }

        Some(Self {
            data: (sample & 0xFF) as u8,
            addr: ((sample >> 8) & 0xFF) as u8,
            read: ((sample >> READ_WRITE_PIN) & 1) != 0,
        })
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
        if is_packed_bcd(response) {
            self.pairs[idx] = response;
            self.pair_mask |= 1 << idx;
        } else {
            self.pairs[idx] = 0;
            self.pair_mask |= 1 << idx;
        }
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

pub struct FeedbackCommand {
    pub index: u64,
    pub cmd: u8,
    pub value: u8,
}

impl FeedbackCommand {
    pub fn from_bytes(index: u64, cmd: u8, value: u8) -> Self {
        Self { cmd, value, index }
    }
}

pub struct FeedbackDecoder {
    pending_cmd: Option<u8>,
    x: AxisState,
    z: AxisState,
    rpm_pairs: [u8; 2],
    rpm_mask: u8,
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
        }
    }

    pub fn ingest_sample(
        &mut self,
        sample_index: u64,
        sample: u32,
    ) -> Result<FeedbackSnapshot, &str> {
        if let Some(cycle) = TraceCycle::from_sample(sample) {
            return self.ingest_cycle(sample_index, cycle);
        }
        Err("None from from_sample")
    }

    pub fn ingest_cycle(
        &mut self,
        sample_index: u64,
        cycle: TraceCycle,
    ) -> Result<FeedbackSnapshot, &str> {
        if cycle.addr == 0x80 && !cycle.read {
            self.pending_cmd = Some(cycle.data);
            return Err("recorded pending_cmd");
        }

        if cycle.addr != 0xF1 || !cycle.read {
            return Err("not 0xF1 / not read");
        }

        if let Some(cmd) = self.pending_cmd.take() {
            self.ingest_command(FeedbackCommand::from_bytes(sample_index, cmd, cycle.data))
        } else {
            Err("no pending_cmd")
        }
    }

    pub fn ingest_command(&mut self, command: FeedbackCommand) -> Result<FeedbackSnapshot, &str> {
        match command.cmd {
            0x03 => self.x.set_sign(command.value),
            0x02 => self.x.set_pair(0, command.value),
            0x01 => self.x.set_pair(1, command.value),
            0x00 => self.x.set_pair(2, command.value),
            0x07 => self.z.set_sign(command.value),
            0x06 => self.z.set_pair(0, command.value),
            0x05 => self.z.set_pair(1, command.value),
            0x04 => self.z.set_pair(2, command.value),
            0x0D => {
                if is_packed_bcd(command.value) {
                    self.rpm_pairs[0] = command.value;
                    self.rpm_mask |= 1 << 0;
                } else {
                    self.rpm_pairs[0] = 0;
                    self.rpm_mask |= 1 << 0;
                }
            }
            0x0C => {
                if is_packed_bcd(command.value) {
                    self.rpm_pairs[1] = command.value;
                    self.rpm_mask |= 1 << 1;
                } else {
                    self.rpm_pairs[1] = 0;
                    self.rpm_mask |= 1 << 1;
                }
            }
            _ => {}
        }

        if command.cmd != 0x0C {
            return Err("no 0x0C yet");
        }

        match self.snapshot(command.index) {
            Ok(snapshot) => {
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    }

    fn snapshot(&self, sample_index: u64) -> Result<FeedbackSnapshot, &str> {
        if self.rpm_mask != 0b11 {
            return Err("incomplete RPM mask");
        }

        let x = match self.x.snapshot() {
            Some(x) => x,
            None => {
                return Err("no x snapshot");
            }
        };
        let z = match self.z.snapshot() {
            Some(z) => z,
            None => {
                return Err("no z snapshot");
            }
        };
        let rpm_raw =
            (bcd_pair_value(self.rpm_pairs[0]) * 100 + bcd_pair_value(self.rpm_pairs[1])) as u16;

        Ok(FeedbackSnapshot {
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
