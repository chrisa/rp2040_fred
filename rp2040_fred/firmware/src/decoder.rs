use crate::{decoder::{axis::{AxisSnapshot, AxisState}, spindle::{SpindleSnapshot, SpindleState}}, resources::{FRED_PIN, ONE_MHZ_PIN, READ_WRITE_PIN}};

mod axis;
mod spindle;

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
    pub s: SpindleSnapshot,
}

impl Default for FeedbackSnapshot {
    fn default() -> Self {
        Self {
            sample_index: 0,
            x: AxisSnapshot::default(),
            z: AxisSnapshot::default(),
            s: SpindleSnapshot::default(),
        }
    }
}

pub struct FeedbackCommand {
    pub index: u64,
    pub cmd: u8,
    pub value: u8,
    pub rpm_trigger: bool,
}

impl FeedbackCommand {
    pub fn from_master(index: u64, cmd: u8, value: u8, rpm_trigger: bool) -> Self {
        Self { cmd, value, index, rpm_trigger }
    }

    pub fn from_cycle(index: u64, cmd: u8, value: u8) -> Self {
        Self { cmd, value, index, rpm_trigger: false }
    }
}

pub struct FeedbackDecoder {
    pending_cmd: Option<u8>,
    x: AxisState,
    z: AxisState,
    s: SpindleState,
    last_s: SpindleSnapshot,
}

impl Default for FeedbackDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackDecoder {
    pub fn new() -> Self {
        Self {
            pending_cmd: None,
            x: AxisState::default(),
            z: AxisState::default(),
            s: SpindleState::default(),
            last_s: SpindleSnapshot::default(),
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
            self.ingest_command(FeedbackCommand::from_cycle(sample_index, cmd, cycle.data))
        } else {
            Err("no pending_cmd")
        }
    }

    pub fn ingest_command(&mut self, command: FeedbackCommand) -> Result<FeedbackSnapshot, &str> {
        match command.cmd {
            0x03 => {
                self.x.reset();
                self.x.set_sign(command.value);
            },
            0x02 => self.x.set_pair(0, command.value),
            0x01 => self.x.set_pair(1, command.value),
            0x00 => self.x.set_pair(2, command.value),
            0x07 => {
                self.z.reset();
                self.z.set_sign(command.value);
            },
            0x06 => self.z.set_pair(0, command.value),
            0x05 => self.z.set_pair(1, command.value),
            0x04 => self.z.set_pair(2, command.value),
            0x0D => {
                self.s.reset();
                self.s.set_pair(0, command.value);
            },
            0x0C => self.s.set_pair(1, command.value),
            _ => {}
        }

        if command.rpm_trigger {
            self.s.trigger();
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

    fn snapshot(&mut self, sample_index: u64) -> Result<FeedbackSnapshot, &str> {
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
        let s = match self.s.snapshot() {
            Some(s) => s,
            None => self.last_s,
        };

        self.last_s = s;

        Ok(FeedbackSnapshot {
            sample_index,
            x,
            z,
            s,
        })
    }
}

fn is_packed_bcd(byte: u8) -> bool {
    (byte >> 4) <= 9 && (byte & 0x0F) <= 9
}

fn bcd_pair_value(byte: u8) -> u32 {
    ((byte >> 4) as u32) * 10 + (byte & 0x0F) as u32
}
