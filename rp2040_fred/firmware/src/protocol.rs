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
            // X sign + 6 hex digits across 02/01/00
            0x03 => sign_byte(self.telemetry.x_counts),
            0x02 => byte2(abs24(self.telemetry.x_counts)),
            0x01 => byte1(abs24(self.telemetry.x_counts)),
            0x00 => byte0(abs24(self.telemetry.x_counts)),

            // Z sign + 6 hex digits across 06/05/04
            0x07 => sign_byte(self.telemetry.z_counts),
            0x06 => byte2(abs24(self.telemetry.z_counts)),
            0x05 => byte1(abs24(self.telemetry.z_counts)),
            0x04 => byte0(abs24(self.telemetry.z_counts)),

            // Speed digits (ROM callback forces the very last digit to zero on 0x0C path).
            0x0D => (self.telemetry.rpm >> 8) as u8,
            0x0C => (self.telemetry.rpm & 0x00FF) as u8,

            _ => 0x00,
        };

        FredReply {
            status_fcf0: status,
            response_fcf1: response,
        }
    }
}

const fn abs24(v: i32) -> u32 {
    let mag = if v < 0 { (-v) as u32 } else { v as u32 };
    mag & 0x00FF_FFFF
}

const fn sign_byte(v: i32) -> u8 {
    if v < 0 { 0x01 } else { 0x00 }
}

const fn byte2(v: u32) -> u8 {
    ((v >> 16) & 0xFF) as u8
}

const fn byte1(v: u32) -> u8 {
    ((v >> 8) & 0xFF) as u8
}

const fn byte0(v: u32) -> u8 {
    (v & 0xFF) as u8
}
