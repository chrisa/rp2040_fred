use std::io;

use rp2040_fred_protocol::bridge_proto::{CommandBlock, CommandBlockRequest};

pub const SPINDLE_START_REVERSE_SUBCODE: u8 = 3;
pub const SPINDLE_START_FORWARD_SUBCODE: u8 = 4;
pub const SPINDLE_STOP_SUBCODE: u8 = 5;
pub const SPINDLE_MAX_SPEED_CODE: u16 = 127;
pub const SPINDLE_RPM_PER_SPEED_CODE: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpindleDirection {
    Forward,
    Reverse,
}

impl SpindleDirection {
    pub fn command_subcode(self) -> u8 {
        match self {
            Self::Forward => SPINDLE_START_FORWARD_SUBCODE,
            Self::Reverse => SPINDLE_START_REVERSE_SUBCODE,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Reverse => "reverse",
        }
    }
}

pub fn spindle_start_command_block(
    direction: SpindleDirection,
    speed_code: u16,
) -> io::Result<CommandBlock> {
    validate_speed_code(speed_code)?;

    Ok(CommandBlock {
        m1: 0,
        m2: direction.command_subcode(),
        m3: 0,
        m4: 0,
        m5: 0,
        m6: 0,
        m7: 0,
        m8: 0,
        m9: speed_code,
        m10: 0,
    })
}

pub fn spindle_stop_command_block() -> CommandBlock {
    CommandBlock {
        m1: 0,
        m2: SPINDLE_STOP_SUBCODE,
        m3: 0,
        m4: 0,
        m5: 0,
        m6: 0,
        m7: 0,
        m8: 0,
        m9: 0,
        m10: 0,
    }
}

pub fn spindle_start_request(
    direction: SpindleDirection,
    speed_code: u16,
) -> io::Result<CommandBlockRequest> {
    Ok(CommandBlockRequest {
        block: spindle_start_command_block(direction, speed_code)?,
        flags: 0,
    })
}

pub fn spindle_stop_request() -> CommandBlockRequest {
    CommandBlockRequest {
        block: spindle_stop_command_block(),
        flags: 0,
    }
}

pub fn speed_code_from_rpm(rpm: f32) -> io::Result<u16> {
    if !rpm.is_finite() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "spindle RPM must be finite",
        ));
    }

    let rpm = rpm.abs();
    if rpm <= 4.0 {
        return Ok(0);
    }

    let raw = (rpm / SPINDLE_RPM_PER_SPEED_CODE).round();
    let clamped = raw.min(f32::from(SPINDLE_MAX_SPEED_CODE));
    Ok(clamped as u16)
}

pub fn validate_speed_code(speed_code: u16) -> io::Result<()> {
    if speed_code > SPINDLE_MAX_SPEED_CODE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("spindle speed code must be in range 0..={SPINDLE_MAX_SPEED_CODE}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        speed_code_from_rpm, spindle_start_command_block, spindle_stop_command_block,
        SpindleDirection, SPINDLE_MAX_SPEED_CODE, SPINDLE_START_FORWARD_SUBCODE,
        SPINDLE_START_REVERSE_SUBCODE, SPINDLE_STOP_SUBCODE,
    };

    #[test]
    fn reverse_start_block_matches_capture() {
        let block =
            spindle_start_command_block(SpindleDirection::Reverse, 125).expect("spindle start");

        assert_eq!(block.m1, 0);
        assert_eq!(block.m2, SPINDLE_START_REVERSE_SUBCODE);
        assert_eq!(block.m3, 0);
        assert_eq!(block.m8, 0);
        assert_eq!(block.m9, 125);
        assert_eq!(block.m10, 0);
    }

    #[test]
    fn forward_start_block_uses_inferred_subcode() {
        let block =
            spindle_start_command_block(SpindleDirection::Forward, 125).expect("spindle start");

        assert_eq!(block.m1, 0);
        assert_eq!(block.m2, SPINDLE_START_FORWARD_SUBCODE);
        assert_eq!(block.m9, 125);
    }

    #[test]
    fn stop_block_matches_capture() {
        let block = spindle_stop_command_block();

        assert_eq!(block.m1, 0);
        assert_eq!(block.m2, SPINDLE_STOP_SUBCODE);
        assert_eq!(block.m3, 0);
        assert_eq!(block.m9, 0);
        assert_eq!(block.m10, 0);
    }

    #[test]
    fn rpm_to_speed_code_matches_captured_s3000_case() {
        assert_eq!(speed_code_from_rpm(3000.0).expect("speed code"), 125);
    }

    #[test]
    fn rpm_to_speed_code_caps_at_old_host_limit() {
        assert_eq!(
            speed_code_from_rpm(10000.0).expect("speed code"),
            SPINDLE_MAX_SPEED_CODE
        );
    }

    #[test]
    fn rpm_to_speed_code_rejects_nonfinite_values() {
        assert_eq!(
            speed_code_from_rpm(f32::NAN).expect_err("nonfinite").kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn start_block_rejects_out_of_range_speed_code() {
        assert_eq!(
            spindle_start_command_block(SpindleDirection::Forward, SPINDLE_MAX_SPEED_CODE + 1)
                .expect_err("bad speed code")
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
