use std::io;
use std::thread;
use std::time::{Duration, Instant};

use rp2040_fred_protocol::bridge_proto::{CommandBlockRequest, ControllerStatus, MsgType, Packet};

use crate::motion::{self, AxisCalibration};
use crate::spindle::{self, SpindleDirection};
use crate::transport::{UsbRole, UsbTransport};

const IDLE_READ_TIMEOUT: Duration = Duration::from_millis(1000);
const SHORT_READ_TIMEOUT: Duration = Duration::from_millis(1);
const TRANSACT_TIMEOUT: Duration = Duration::from_millis(5000);
const WAIT_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DroSnapshot {
    pub x_counts: i32,
    pub z_counts: i32,
    pub rpm: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Calibration {
    pub x_counts_per_mm: f32,
    pub z_counts_per_mm: f32,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            x_counts_per_mm: 100.0,
            z_counts_per_mm: 100.0,
        }
    }
}

pub fn counts_to_mm(snapshot: DroSnapshot, cal: Calibration) -> (f32, f32, u16) {
    // CNCMAN uses diameter semantics for X (x*2), direct for Z.
    let x_mm = ((snapshot.x_counts as f32) * 2.0) / cal.x_counts_per_mm;
    let z_mm = (snapshot.z_counts as f32) / cal.z_counts_per_mm;
    (x_mm, z_mm, snapshot.rpm)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorSnapshot {
    pub x_mm: f32,
    pub z_mm: f32,
    pub spindle_rpm: u16,
    pub x_counts: i32,
    pub z_counts: i32,
    pub tick: u32,
    pub generation: u32,
    pub flags: u8,
}

impl Default for MonitorSnapshot {
    fn default() -> Self {
        Self {
            x_mm: 0.0,
            z_mm: 0.0,
            spindle_rpm: 0,
            x_counts: 0,
            z_counts: 0,
            tick: 0,
            generation: 0,
            flags: 0,
        }
    }
}

impl MonitorSnapshot {
    pub fn from_telemetry_packet(pkt: &Packet, calibration: Calibration) -> Option<Self> {
        if pkt.msg_type != MsgType::Telemetry || pkt.payload_len < 16 {
            return None;
        }

        let payload = pkt.payload_used();
        let snapshot = DroSnapshot {
            x_counts: i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]),
            z_counts: i32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]),
            rpm: u16::from_le_bytes([payload[12], payload[13]]),
        };
        let (x_mm, z_mm, spindle_rpm) = counts_to_mm(snapshot, calibration);

        Some(Self {
            x_mm,
            z_mm,
            spindle_rpm,
            x_counts: snapshot.x_counts,
            z_counts: snapshot.z_counts,
            tick: u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]),
            generation: 0,
            flags: payload[14],
        })
    }
}

pub struct FredMonitorClient {
    transport: UsbTransport,
    calibration: Calibration,
    latest: MonitorSnapshot,
    latest_valid: bool,
    generation: u32,
    seq: u16,
}

impl FredMonitorClient {
    pub fn open(vid: u16, pid: u16) -> io::Result<Self> {
        Self::open_with_options(
            vid,
            pid,
            Duration::from_millis(1000),
            Calibration::default(),
        )
    }

    pub fn open_with_options(
        vid: u16,
        pid: u16,
        timeout: Duration,
        calibration: Calibration,
    ) -> io::Result<Self> {
        let mut transport = UsbTransport::open(vid, pid, UsbRole::Master)?;
        transport.set_timeout(timeout);
        Ok(Self {
            transport,
            calibration,
            latest: MonitorSnapshot::default(),
            latest_valid: false,
            generation: 0,
            seq: 1,
        })
    }

    pub fn enable_polling(&mut self, period_ms: u16) -> io::Result<()> {
        let seq = self.next_seq();
        self.transact_expect_ack(
            Packet::telemetry_set(seq, true, period_ms),
            MsgType::TelemetrySet,
        )?;
        Ok(())
    }

    pub fn disable_polling(&mut self) -> io::Result<()> {
        let seq = self.next_seq();
        self.transact_expect_ack(Packet::telemetry_set(seq, false, 0), MsgType::TelemetrySet)?;
        Ok(())
    }

    pub fn refresh(&mut self) -> io::Result<MonitorSnapshot> {
        let _ = self.refresh_timeout(IDLE_READ_TIMEOUT)?;
        Ok(self.latest)
    }

    pub fn refresh_timeout(&mut self, timeout: Duration) -> io::Result<Option<MonitorSnapshot>> {
        let deadline = Instant::now() + timeout;
        let mut latest_update = None;

        loop {
            let read_timeout = if timeout.is_zero() {
                SHORT_READ_TIMEOUT
            } else {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(latest_update);
                }
                remaining.max(SHORT_READ_TIMEOUT)
            };

            match self.transport.read_packet_timeout(read_timeout) {
                Ok(pkt) => {
                    if self.consume_packet(&pkt) {
                        latest_update = self.latest();
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::TimedOut => return Ok(latest_update),
                Err(err) => return Err(err),
            }
        }
    }

    pub fn next_snapshot(&mut self) -> io::Result<MonitorSnapshot> {
        loop {
            let pkt = self.transport.read_packet()?;
            if self.consume_packet(&pkt) {
                return Ok(self.latest);
            }
        }
    }

    pub fn latest(&self) -> Option<MonitorSnapshot> {
        self.latest_valid.then_some(self.latest)
    }

    pub fn rapid_move_delta_mm(
        &mut self,
        x_mm: f32,
        z_mm: f32,
        slew: u16,
        wait: bool,
    ) -> io::Result<bool> {
        let calibration = self.axis_calibration();
        let Some(request) = motion::rapid_command_request_mm(x_mm, z_mm, slew, calibration)? else {
            return Ok(false);
        };

        self.send_command_request(request)?;
        if wait {
            self.wait_idle(None)?;
        }
        Ok(true)
    }

    pub fn feed_move_delta_mm(
        &mut self,
        x_mm: f32,
        z_mm: f32,
        feed: u32,
        slew: u16,
        wait: bool,
    ) -> io::Result<bool> {
        let calibration = self.axis_calibration();
        let Some(request) = motion::feed_command_request_mm(x_mm, z_mm, feed, slew, calibration)?
        else {
            return Ok(false);
        };

        self.send_command_request(request)?;
        if wait {
            self.wait_idle(None)?;
        }
        Ok(true)
    }

    pub fn set_spindle(
        &mut self,
        on: bool,
        rpm: f32,
        forward: bool,
        speed_code: Option<u16>,
        wait: bool,
    ) -> io::Result<bool> {
        let request = if on {
            let direction = if forward {
                SpindleDirection::Forward
            } else {
                SpindleDirection::Reverse
            };
            let speed_code = match speed_code {
                Some(speed_code) => {
                    spindle::validate_speed_code(speed_code)?;
                    speed_code
                }
                None => spindle::speed_code_from_rpm(rpm)?,
            };
            spindle::spindle_start_request(direction, speed_code)?
        } else {
            spindle::spindle_stop_request()
        };

        self.send_command_request(request)?;
        if wait {
            self.wait_idle(None)?;
        }
        Ok(true)
    }

    pub fn send_command_request(&mut self, request: CommandBlockRequest) -> io::Result<()> {
        let seq = self.next_seq();
        self.transact_expect_ack(
            Packet::command_block_request(seq, request),
            MsgType::CommandBlock,
        )
    }

    pub fn controller_status(&mut self) -> io::Result<ControllerStatus> {
        let seq = self.next_seq();
        let req = Packet::controller_status_req(seq);
        self.transport.write_packet(&req)?;

        let deadline = Instant::now() + TRANSACT_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "no controller status response",
                ));
            }

            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .max(SHORT_READ_TIMEOUT);
            let pkt = self.transport.read_packet_timeout(timeout)?;

            if self.consume_packet(&pkt) {
                continue;
            }
            if pkt.seq != seq {
                continue;
            }
            if let Some(status) = pkt.decode_controller_status_ack() {
                return Ok(status);
            }
            if pkt.msg_type == MsgType::Nack
                && pkt.payload_len >= 2
                && pkt.payload[0] == MsgType::ControllerStatusReq as u8
            {
                return Err(io::Error::other(format!(
                    "device rejected ControllerStatusReq with reason {:#04x}",
                    pkt.payload[1]
                )));
            }
        }
    }

    pub fn wait_idle(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        let deadline = timeout.map(|duration| Instant::now() + duration);

        loop {
            let status = self.controller_status()?;
            if status.has_error() {
                return Err(io::Error::other("controller work reported an error"));
            }
            if status.is_idle() {
                return Ok(());
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "controller did not become idle before timeout",
                ));
            }
            thread::sleep(WAIT_IDLE_POLL_INTERVAL);
        }
    }

    pub fn close(self) {}

    fn next_seq(&mut self) -> u16 {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1).max(1);
        seq
    }

    fn axis_calibration(&self) -> AxisCalibration {
        AxisCalibration {
            x_counts_per_mm: self.calibration.x_counts_per_mm,
            z_counts_per_mm: self.calibration.z_counts_per_mm,
        }
    }

    fn transact_expect_ack(&mut self, req: Packet, acked_type: MsgType) -> io::Result<()> {
        let want_seq = req.seq;
        self.transport.write_packet(&req)?;

        let deadline = Instant::now() + TRANSACT_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("no ACK/NACK for {:?}", acked_type),
                ));
            }

            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .max(SHORT_READ_TIMEOUT);
            let pkt = self.transport.read_packet_timeout(timeout)?;

            if self.consume_packet(&pkt) {
                continue;
            }
            if pkt.seq != want_seq {
                continue;
            }

            match pkt.msg_type {
                MsgType::Ack if pkt.payload_len >= 2 && pkt.payload[0] == acked_type as u8 => {
                    let status = pkt.payload[1];
                    if status == 0 {
                        return Ok(());
                    }
                    return Err(io::Error::other(format!(
                        "device acked {:?} with nonzero status {status:#04x}",
                        acked_type
                    )));
                }
                MsgType::Nack if pkt.payload_len >= 2 && pkt.payload[0] == acked_type as u8 => {
                    return Err(io::Error::other(format!(
                        "device rejected {:?} with reason {:#04x}",
                        acked_type, pkt.payload[1]
                    )));
                }
                _ => {}
            }
        }
    }

    fn consume_packet(&mut self, pkt: &Packet) -> bool {
        let Some(mut snapshot) = MonitorSnapshot::from_telemetry_packet(pkt, self.calibration)
        else {
            return false;
        };
        self.generation = self.generation.wrapping_add(1);
        snapshot.generation = self.generation;
        self.latest = snapshot;
        self.latest_valid = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{Calibration, MonitorSnapshot};
    use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};

    #[test]
    fn telemetry_packet_decodes_to_monitor_snapshot() {
        let packet = Packet::telemetry(9, 123, -100, 250, 780, 0x5A);
        let snapshot =
            MonitorSnapshot::from_telemetry_packet(&packet, Calibration::default()).expect("valid");

        assert_eq!(snapshot.tick, 123);
        assert_eq!(snapshot.x_counts, -100);
        assert_eq!(snapshot.z_counts, 250);
        assert_eq!(snapshot.spindle_rpm, 780);
        assert_eq!(snapshot.generation, 0);
        assert_eq!(snapshot.flags, 0x5A);
        assert!((snapshot.x_mm + 2.0).abs() < 0.0001);
        assert!((snapshot.z_mm - 2.5).abs() < 0.0001);
    }

    #[test]
    fn non_telemetry_packets_are_ignored() {
        let packet = Packet::ack(7, MsgType::TelemetrySet, 0);
        assert!(MonitorSnapshot::from_telemetry_packet(&packet, Calibration::default()).is_none());
    }
}
