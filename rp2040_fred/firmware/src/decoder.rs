use crate::{
    decoder::{
        axis::{AxisSnapshot, AxisState},
        spindle::{SpindleSnapshot, SpindleState},
    },
    resources::{FRED_PIN, ONE_MHZ_PIN, READ_WRITE_PIN},
};

#[path = "decoder/axis.rs"]
mod axis;
#[path = "decoder/spindle.rs"]
mod spindle;

const FEEDBACK_COMMANDS_PER_POSITION_SNAPSHOT: u64 = 10;
const MAX_POSITION_CHANGE_COUNTS_PER_SNAPSHOT: u64 = 2_000;

// Compile-time switch for incomplete BCD snapshots at the 0x0C boundary.
// Current preserves the existing behavior; HoldPrevious and DropIncomplete are
// the two explicit strategies for noisy non-BCD reads.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "selected manually while tuning noisy feedback")
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MissingBcdSnapshotPolicy {
    Current,
    HoldPrevious,
    DropIncomplete,
}

const MISSING_BCD_SNAPSHOT_POLICY: MissingBcdSnapshotPolicy = MissingBcdSnapshotPolicy::Current;

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
        Self {
            cmd,
            value,
            index,
            rpm_trigger,
        }
    }

    pub fn from_cycle(index: u64, cmd: u8, value: u8) -> Self {
        Self {
            cmd,
            value,
            index,
            rpm_trigger: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AcceptedAxis {
    snapshot: AxisSnapshot,
    sample_index: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AxisResolution {
    snapshot: AxisSnapshot,
    accepted: Option<AcceptedAxis>,
}

pub struct FeedbackDecoder {
    pending_cmd: Option<u8>,
    x: AxisState,
    z: AxisState,
    s: SpindleState,
    last_x: Option<AcceptedAxis>,
    last_z: Option<AcceptedAxis>,
    last_s: SpindleSnapshot,
    last_s_valid: bool,
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
            last_x: None,
            last_z: None,
            last_s: SpindleSnapshot::default(),
            last_s_valid: false,
        }
    }

    pub fn ingest_sample(
        &mut self,
        sample_index: u64,
        sample: u32,
    ) -> Result<FeedbackSnapshot, &'static str> {
        if let Some(cycle) = TraceCycle::from_sample(sample) {
            return self.ingest_cycle(sample_index, cycle);
        }
        Err("None from from_sample")
    }

    pub fn ingest_cycle(
        &mut self,
        sample_index: u64,
        cycle: TraceCycle,
    ) -> Result<FeedbackSnapshot, &'static str> {
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

    pub fn ingest_command(
        &mut self,
        command: FeedbackCommand,
    ) -> Result<FeedbackSnapshot, &'static str> {
        self.ingest_command_with_policy(command, MISSING_BCD_SNAPSHOT_POLICY)
    }

    pub(crate) fn ingest_command_with_policy(
        &mut self,
        command: FeedbackCommand,
        policy: MissingBcdSnapshotPolicy,
    ) -> Result<FeedbackSnapshot, &'static str> {
        match command.cmd {
            0x03 => {
                self.x.reset();
                self.x.set_sign(command.value);
            }
            0x02 => self.x.set_pair(0, command.value),
            0x01 => self.x.set_pair(1, command.value),
            0x00 => self.x.set_pair(2, command.value),
            0x07 => {
                self.z.reset();
                self.z.set_sign(command.value);
            }
            0x06 => self.z.set_pair(0, command.value),
            0x05 => self.z.set_pair(1, command.value),
            0x04 => self.z.set_pair(2, command.value),
            0x0D => {
                self.s.reset();
                self.s.set_pair(0, command.value);
            }
            0x0C => self.s.set_pair(1, command.value),
            _ => {}
        }

        if command.rpm_trigger {
            self.s.trigger();
        }

        if command.cmd != 0x0C {
            return Err("no 0x0C yet");
        }

        match self.snapshot_with_policy(command.index, policy) {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => Err(error),
        }
    }

    fn snapshot_with_policy(
        &mut self,
        sample_index: u64,
        policy: MissingBcdSnapshotPolicy,
    ) -> Result<FeedbackSnapshot, &'static str> {
        let x = resolve_axis(
            self.last_x,
            self.x.snapshot(),
            sample_index,
            policy,
            "no x snapshot",
            "no previous x snapshot",
        )?;
        let z = resolve_axis(
            self.last_z,
            self.z.snapshot(),
            sample_index,
            policy,
            "no z snapshot",
            "no previous z snapshot",
        )?;
        let raw_s = self.s.snapshot();
        let s = resolve_spindle(self.last_s, self.last_s_valid, raw_s, policy)?;

        if let Some(accepted) = x.accepted {
            self.last_x = Some(accepted);
        }
        if let Some(accepted) = z.accepted {
            self.last_z = Some(accepted);
        }
        if let Some(raw_s) = raw_s {
            self.last_s = raw_s;
            self.last_s_valid = true;
        }

        Ok(FeedbackSnapshot {
            sample_index,
            x: x.snapshot,
            z: z.snapshot,
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

fn resolve_axis(
    last: Option<AcceptedAxis>,
    candidate: Option<AxisSnapshot>,
    sample_index: u64,
    policy: MissingBcdSnapshotPolicy,
    missing_error: &'static str,
    missing_previous_error: &'static str,
) -> Result<AxisResolution, &'static str> {
    let Some(candidate) = candidate else {
        return match policy {
            MissingBcdSnapshotPolicy::Current | MissingBcdSnapshotPolicy::DropIncomplete => {
                Err(missing_error)
            }
            MissingBcdSnapshotPolicy::HoldPrevious => {
                let Some(previous) = last else {
                    return Err(missing_previous_error);
                };
                Ok(AxisResolution {
                    snapshot: previous.snapshot,
                    accepted: None,
                })
            }
        };
    };

    let accepted = AcceptedAxis {
        snapshot: candidate,
        sample_index,
    };

    let Some(previous) = last else {
        return Ok(AxisResolution {
            snapshot: candidate,
            accepted: Some(accepted),
        });
    };

    if position_change_allowed(previous, candidate, sample_index) {
        Ok(AxisResolution {
            snapshot: candidate,
            accepted: Some(accepted),
        })
    } else {
        Ok(AxisResolution {
            snapshot: previous.snapshot,
            accepted: None,
        })
    }
}

fn resolve_spindle(
    last: SpindleSnapshot,
    last_valid: bool,
    candidate: Option<SpindleSnapshot>,
    policy: MissingBcdSnapshotPolicy,
) -> Result<SpindleSnapshot, &'static str> {
    match candidate {
        Some(candidate) => Ok(candidate),
        None => match policy {
            MissingBcdSnapshotPolicy::Current => Ok(last),
            MissingBcdSnapshotPolicy::HoldPrevious => {
                if last_valid {
                    Ok(last)
                } else {
                    Err("no previous spindle snapshot")
                }
            }
            MissingBcdSnapshotPolicy::DropIncomplete => Err("no spindle snapshot"),
        },
    }
}

fn position_change_allowed(
    previous: AcceptedAxis,
    candidate: AxisSnapshot,
    sample_index: u64,
) -> bool {
    let previous_count = i64::from(previous.snapshot.count());
    let candidate_count = i64::from(candidate.count());
    let delta = if candidate_count >= previous_count {
        (candidate_count - previous_count) as u64
    } else {
        (previous_count - candidate_count) as u64
    };

    let command_delta = sample_index.saturating_sub(previous.sample_index);
    let snapshots_elapsed = (command_delta / FEEDBACK_COMMANDS_PER_POSITION_SNAPSHOT).max(1);
    let allowed = MAX_POSITION_CHANGE_COUNTS_PER_SNAPSHOT.saturating_mul(snapshots_elapsed);

    delta <= allowed
}
