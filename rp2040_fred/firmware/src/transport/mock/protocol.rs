#![allow(dead_code)]

#[derive(Clone, Copy, Debug)]
pub struct DroTelemetry {
    pub x_counts: i32,
    pub z_counts: i32,
    pub rpm: u16,
}

impl Default for DroTelemetry {
    fn default() -> Self {
        Self {
            x_counts: 0,
            z_counts: 0,
            rpm: 1200,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FredReply {
    pub status_fcf0: u8,
    pub response_fcf1: u8,
}

pub struct DroProtocolEngine {
    telemetry: DroTelemetry,
    tick: u32,
}

impl DroProtocolEngine {
    pub const fn new() -> Self {
        Self {
            telemetry: DroTelemetry {
                x_counts: 0,
                z_counts: 0,
                rpm: 1200,
            },
            tick: 0,
        }
    }

    pub fn step_telemetry(&mut self) {
        self.tick = self.tick.wrapping_add(1);

        // Deterministic synthetic trajectory for first bring-up.
        let phase = (self.tick >> 4) as i32;
        self.telemetry.x_counts = (phase & 0x03FF) - 0x0200;
        self.telemetry.z_counts = ((phase * 3) & 0x03FF) - 0x0200;
        self.telemetry.rpm = 800 + ((phase as u16) & 0x00FF) * 5;
    }

    pub fn on_command(&mut self, cmd_fc80: u8) -> FredReply {
        // Current status model: always ready.
        let status = 0x00;

        let response = match cmd_fc80 {
            // X sign + 6 decimal digits split into packed BCD pairs at 02/01/00.
            0x03 => sign_byte(self.telemetry.x_counts),
            0x02 => axis_pair(self.telemetry.x_counts, 0),
            0x01 => axis_pair(self.telemetry.x_counts, 1),
            0x00 => axis_pair(self.telemetry.x_counts, 2),

            // Z sign + 6 decimal digits split into packed BCD pairs at 06/05/04.
            0x07 => sign_byte(self.telemetry.z_counts),
            0x06 => axis_pair(self.telemetry.z_counts, 0),
            0x05 => axis_pair(self.telemetry.z_counts, 1),
            0x04 => axis_pair(self.telemetry.z_counts, 2),

            // RPM as two packed BCD pairs.
            0x0D => rpm_pair(self.telemetry.rpm, 0),
            0x0C => rpm_pair(self.telemetry.rpm, 1),

            _ => 0x00,
        };

        FredReply {
            status_fcf0: status,
            response_fcf1: response,
        }
    }
}

const fn sign_byte(v: i32) -> u8 {
    if v < 0 {
        0x01
    } else {
        0x00
    }
}

fn axis_pair(counts: i32, pair_index: usize) -> u8 {
    let mag = counts.unsigned_abs().min(999_999);
    let pair_value = match pair_index {
        0 => mag / 10_000,
        1 => (mag / 100) % 100,
        _ => mag % 100,
    };
    pack_bcd(pair_value as u8)
}

fn rpm_pair(rpm: u16, pair_index: usize) -> u8 {
    let rpm = rpm.min(9_999);
    let pair_value = match pair_index {
        0 => rpm / 100,
        _ => rpm % 100,
    };
    pack_bcd(pair_value as u8)
}

const fn pack_bcd(two_digits: u8) -> u8 {
    ((two_digits / 10) << 4) | (two_digits % 10)
}
