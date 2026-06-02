use std::io;

use rp2040_fred_protocol::bridge_proto::{CommandBlock, CommandBlockRequest};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisCalibration {
    pub x_counts_per_mm: f32,
    pub z_counts_per_mm: f32,
}

pub fn rapid_command_request_mm(
    x_mm: f32,
    z_mm: f32,
    slew: u16,
    calibration: AxisCalibration,
) -> io::Result<Option<CommandBlockRequest>> {
    let x_counts = x_diameter_counts_from_mm(x_mm, calibration.x_counts_per_mm)?;
    let z_counts = z_counts_from_mm(z_mm, calibration.z_counts_per_mm)?;

    if x_counts == 0 && z_counts == 0 {
        return Ok(None);
    }

    Ok(Some(CommandBlockRequest {
        block: rapid_command_block(x_counts, z_counts, slew)?,
        flags: 0,
    }))
}

pub fn feed_command_request_mm(
    x_mm: f32,
    z_mm: f32,
    feed: u32,
    slew: u16,
    calibration: AxisCalibration,
) -> io::Result<Option<CommandBlockRequest>> {
    let Some(mut request) = rapid_command_request_mm(x_mm, z_mm, slew, calibration)? else {
        return Ok(None);
    };

    request.block.m1 = 1;
    request.block.m8 = feed_timing(feed)?;
    Ok(Some(request))
}

pub fn rapid_command_block(
    x_diameter_counts: i32,
    z_counts: i32,
    slew: u16,
) -> io::Result<CommandBlock> {
    let x_radius_counts = checked_x_radius_counts(x_diameter_counts)?;
    let z_counts = checked_i16("z counts", z_counts)?;
    Ok(CommandBlock {
        m1: 0,
        m2: 0,
        m3: raw_word(x_radius_counts),
        m4: raw_word(z_counts),
        m5: 0,
        m6: 0,
        m7: 0,
        m8: 0,
        m9: slew,
        m10: 0,
    })
}

pub fn feed_timing(feed: u32) -> io::Result<u16> {
    if feed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "feed must be greater than zero",
        ));
    }

    let timing = 95_000 / feed;
    if timing == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "feed is too high for nonzero controller timing",
        ));
    }

    u16::try_from(timing).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("feed timing is outside 16-bit range: {timing}"),
        )
    })
}

fn x_diameter_counts_from_mm(delta_mm: f32, counts_per_mm: f32) -> io::Result<i32> {
    let counts = rounded_counts("x delta", delta_mm, counts_per_mm)?;
    if counts % 2 == 0 {
        Ok(counts)
    } else if counts > 0 {
        counts
            .checked_add(1)
            .ok_or_else(|| count_overflow("x delta"))
    } else {
        counts
            .checked_sub(1)
            .ok_or_else(|| count_overflow("x delta"))
    }
}

fn z_counts_from_mm(delta_mm: f32, counts_per_mm: f32) -> io::Result<i32> {
    rounded_counts("z delta", delta_mm, counts_per_mm)
}

fn rounded_counts(label: &str, delta_mm: f32, counts_per_mm: f32) -> io::Result<i32> {
    if !delta_mm.is_finite() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} must be finite"),
        ));
    }
    if !counts_per_mm.is_finite() || counts_per_mm <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} counts/mm must be finite and greater than zero"),
        ));
    }

    let raw = (delta_mm * counts_per_mm).round();
    if raw < i32::MIN as f32 || raw > i32::MAX as f32 {
        return Err(count_overflow(label));
    }
    Ok(raw as i32)
}

fn checked_x_radius_counts(x_diameter_counts: i32) -> io::Result<i16> {
    if x_diameter_counts % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "x diameter counts must be even because the controller uses radius counts",
        ));
    }
    checked_i16("x diameter counts / 2", x_diameter_counts / 2)
}

fn checked_i16(label: &str, value: i32) -> io::Result<i16> {
    i16::try_from(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} is outside signed 16-bit range: {value}"),
        )
    })
}

fn count_overflow(label: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{label} is outside signed 32-bit count range"),
    )
}

fn raw_word(value: i16) -> u16 {
    u16::from_le_bytes(value.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::{
        feed_command_request_mm, feed_timing, rapid_command_block, rapid_command_request_mm,
        raw_word, AxisCalibration,
    };

    fn cal() -> AxisCalibration {
        AxisCalibration {
            x_counts_per_mm: 100.0,
            z_counts_per_mm: 100.0,
        }
    }

    #[test]
    fn rapid_command_block_uses_radius_x_and_direct_z() {
        let block = rapid_command_block(-252, 1500, 61).expect("rapid block");

        assert_eq!(block.m1, 0);
        assert_eq!(block.m2, 0);
        assert_eq!(block.m3, raw_word(-126));
        assert_eq!(block.m4, raw_word(1500));
        assert_eq!(block.m8, 0);
        assert_eq!(block.m9, 61);
        assert_eq!(block.m10, 0);
    }

    #[test]
    fn rapid_mm_request_rounds_x_to_even_diameter_counts() {
        let request = rapid_command_request_mm(0.015, -0.02, 61, cal())
            .expect("request")
            .expect("nonzero move");

        assert_eq!(request.block.m3, raw_word(1));
        assert_eq!(request.block.m4, raw_word(-2));
    }

    #[test]
    fn zero_mm_request_is_skipped() {
        assert_eq!(
            rapid_command_request_mm(0.0, 0.0, 61, cal()).expect("request"),
            None
        );
    }

    #[test]
    fn feed_mm_request_sets_opcode_and_timing() {
        let request = feed_command_request_mm(0.0, 1.0, 100, 61, cal())
            .expect("request")
            .expect("nonzero move");

        assert_eq!(request.block.m1, 1);
        assert_eq!(request.block.m8, 950);
    }

    #[test]
    fn feed_timing_rejects_zero_feed() {
        assert_eq!(
            feed_timing(0).expect_err("zero feed").kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
