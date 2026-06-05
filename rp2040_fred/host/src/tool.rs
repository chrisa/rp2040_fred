use std::io;

use rp2040_fred_protocol::bridge_proto::{CommandBlock, CommandBlockRequest};

pub const TURRET_MIN_STATION: u8 = 1;
pub const TURRET_MAX_STATION: u8 = 8;

pub fn turret_command_requests(
    current_station: u8,
    target_station: u8,
    slew: u16,
) -> io::Result<Vec<CommandBlockRequest>> {
    validate_station("current station", current_station)?;
    validate_station("target station", target_station)?;

    let steps = turret_step_count(current_station, target_station);
    if steps == 0 {
        return Ok(Vec::new());
    }

    Ok(vec![
        CommandBlockRequest {
            block: timed_aux_command_block(-832 * i32::from(steps), 400, slew)?,
            flags: 0,
        },
        CommandBlockRequest {
            block: timed_aux_command_block(159 + 68 * i32::from(steps), 300, slew)?,
            flags: 0,
        },
        CommandBlockRequest {
            block: timed_aux_command_block(10, 600, slew)?,
            flags: 0,
        },
    ])
}

pub fn turret_step_count(current_station: u8, target_station: u8) -> u8 {
    if current_station == target_station {
        0
    } else if current_station < target_station {
        target_station - current_station
    } else {
        TURRET_MAX_STATION - (current_station - target_station)
    }
}

pub fn validate_station(label: &str, station: u8) -> io::Result<()> {
    if !(TURRET_MIN_STATION..=TURRET_MAX_STATION).contains(&station) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} must be in range {TURRET_MIN_STATION}..={TURRET_MAX_STATION}"),
        ));
    }
    Ok(())
}

fn timed_aux_command_block(aux_counts: i32, timing: u16, slew: u16) -> io::Result<CommandBlock> {
    Ok(CommandBlock {
        m1: 1,
        m2: 0,
        m3: 0,
        m4: 0,
        m5: raw_word(checked_i16("auxiliary count", aux_counts)?),
        m6: 0,
        m7: 0,
        m8: timing,
        m9: slew,
        m10: 0,
    })
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
    use super::{raw_word, turret_command_requests, turret_step_count};

    #[test]
    fn turret_step_count_wraps_like_cncmak1() {
        assert_eq!(turret_step_count(1, 1), 0);
        assert_eq!(turret_step_count(1, 4), 3);
        assert_eq!(turret_step_count(6, 2), 4);
        assert_eq!(turret_step_count(8, 1), 1);
    }

    #[test]
    fn turret_requests_match_captured_sequence() {
        let requests = turret_command_requests(6, 2, 61).expect("tool blocks");

        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].block.m1, 1);
        assert_eq!(requests[0].block.m2, 0);
        assert_eq!(requests[0].block.m5, raw_word(-832 * 4));
        assert_eq!(requests[0].block.m8, 400);
        assert_eq!(requests[0].block.m9, 61);
        assert_eq!(requests[0].flags, 0);

        assert_eq!(requests[1].block.m5, raw_word(159 + 68 * 4));
        assert_eq!(requests[1].block.m8, 300);
        assert_eq!(requests[2].block.m5, raw_word(10));
        assert_eq!(requests[2].block.m8, 600);
    }

    #[test]
    fn same_station_builds_no_requests() {
        assert!(turret_command_requests(3, 3, 61)
            .expect("tool blocks")
            .is_empty());
    }

    #[test]
    fn turret_requests_reject_bad_station() {
        assert_eq!(
            turret_command_requests(0, 3, 61)
                .expect_err("bad station")
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
