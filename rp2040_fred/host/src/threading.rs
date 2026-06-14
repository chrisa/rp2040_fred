use std::io;

use rp2040_fred_protocol::bridge_proto::{CommandBlock, CommandBlockRequest};

use crate::motion::{self, AxisCalibration};

pub const THREAD_SYNC_OPCODE: u8 = 84;
pub const PITCH_ACCUMULATOR_PER_MM: f64 = 65_536.0;

pub fn thread_sync_command_request_mm(
    z_mm: f32,
    pitch_mm: f32,
    slew: u16,
    calibration: AxisCalibration,
) -> io::Result<CommandBlockRequest> {
    let (_, z_counts) = motion::delta_counts_from_mm(0.0, z_mm, calibration)?;
    thread_sync_command_request_counts(z_counts, pitch_mm, slew)
}

pub fn thread_sync_command_request_counts(
    z_counts: i32,
    pitch_mm: f32,
    slew: u16,
) -> io::Result<CommandBlockRequest> {
    if z_counts == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "thread sync Z delta must be nonzero",
        ));
    }

    Ok(CommandBlockRequest {
        block: thread_sync_command_block(z_counts, pitch_mm, slew)?,
        flags: 0,
    })
}

pub fn thread_sync_command_block(
    z_counts: i32,
    pitch_mm: f32,
    slew: u16,
) -> io::Result<CommandBlock> {
    Ok(CommandBlock {
        m1: THREAD_SYNC_OPCODE,
        m2: 0,
        m3: 0,
        m4: raw_word(checked_i16("thread sync Z counts", z_counts)?),
        m5: 0,
        m6: 0,
        m7: 0,
        m8: 0,
        m9: slew,
        m10: pitch_accumulator(pitch_mm)?,
    })
}

pub fn pitch_accumulator(pitch_mm: f32) -> io::Result<u32> {
    if !pitch_mm.is_finite() || pitch_mm <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "thread pitch must be finite and greater than zero",
        ));
    }

    let raw = f64::from(pitch_mm) * PITCH_ACCUMULATOR_PER_MM;
    let rounded = raw.round();
    if rounded < 1.0 || rounded > f64::from(u32::MAX) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("thread pitch accumulator is outside u32 range: {rounded}"),
        ));
    }

    Ok(rounded as u32)
}

fn checked_i16(label: &str, value: i32) -> io::Result<i16> {
    i16::try_from(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} is outside signed 16-bit range: {value}"),
        )
    })
}

fn raw_word(value: i16) -> u16 {
    u16::from_le_bytes(value.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::{
        pitch_accumulator, raw_word, thread_sync_command_block, thread_sync_command_request_mm,
        THREAD_SYNC_OPCODE,
    };
    use crate::motion::AxisCalibration;

    fn cal() -> AxisCalibration {
        AxisCalibration {
            x_counts_per_mm: 100.0,
            z_counts_per_mm: 100.0,
        }
    }

    #[test]
    fn pitch_mm_uses_g33_accumulator_scale() {
        assert_eq!(pitch_accumulator(1.5).expect("pitch"), 0x0001_8000);
    }

    #[test]
    fn thread_sync_block_matches_decoded_opcode_84_shape() {
        let block = thread_sync_command_block(1500, 1.5, 61).expect("thread block");

        assert_eq!(block.m1, THREAD_SYNC_OPCODE);
        assert_eq!(block.m2, 0);
        assert_eq!(block.m3, 0);
        assert_eq!(block.m4, raw_word(1500));
        assert_eq!(block.m5, 0);
        assert_eq!(block.m6, 0);
        assert_eq!(block.m7, 0);
        assert_eq!(block.m8, 0);
        assert_eq!(block.m9, 61);
        assert_eq!(block.m10, 0x0001_8000);
    }

    #[test]
    fn thread_sync_mm_request_preserves_z_sign() {
        let request = thread_sync_command_request_mm(-15.0, 1.5, 61, cal()).expect("request");

        assert_eq!(request.block.m4, raw_word(-1500));
    }

    #[test]
    fn thread_sync_rejects_zero_z() {
        assert_eq!(
            thread_sync_command_request_mm(0.0, 1.5, 61, cal())
                .expect_err("zero z")
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn thread_sync_rejects_nonpositive_pitch() {
        assert_eq!(
            thread_sync_command_request_mm(1.0, 0.0, 61, cal())
                .expect_err("zero pitch")
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
