use std::io;
use std::time::Duration;

use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};
use rp2040_fred_protocol::dro_decode::{counts_to_mm, Calibration, DroSnapshot};

use crate::transport::{HostTransport, UsbTransport};

const DEFAULT_VID: u16 = 0x2E8A;
const DEFAULT_PID: u16 = 0x000A;
const IDLE_READ_TIMEOUT: Duration = Duration::from_millis(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorSnapshot {
    pub x_mm: f32,
    pub z_mm: f32,
    pub spindle_rpm: u16,
    pub x_counts: i32,
    pub z_counts: i32,
    pub tick: u32,
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
            flags: payload[14],
        })
    }
}

pub struct FredMonitorClient {
    transport: UsbTransport,
    calibration: Calibration,
    latest: MonitorSnapshot,
}

impl FredMonitorClient {
    pub fn open_default() -> io::Result<Self> {
        Self::open(DEFAULT_VID, DEFAULT_PID)
    }

    pub fn open(vid: u16, pid: u16) -> io::Result<Self> {
        Self::open_with_options(vid, pid, Duration::from_millis(250), Calibration::default())
    }

    pub fn open_with_options(
        vid: u16,
        pid: u16,
        timeout: Duration,
        calibration: Calibration,
    ) -> io::Result<Self> {
        let mut transport = UsbTransport::open(vid, pid)?;
        transport.set_timeout(timeout);
        Ok(Self {
            transport,
            calibration,
            latest: MonitorSnapshot::default(),
        })
    }

    pub fn enable_polling(&mut self, period_ms: u16) -> io::Result<()> {
        let _ = self.transport.transact(Packet::capture_set(1, false))?;
        let _ = self
            .transport
            .transact(Packet::telemetry_set(2, true, period_ms))?;
        Ok(())
    }

    pub fn disable_polling(&mut self) -> io::Result<()> {
        let _ = self
            .transport
            .transact(Packet::telemetry_set(1, false, 0))?;
        Ok(())
    }

    pub fn refresh(&mut self) -> io::Result<MonitorSnapshot> {
        loop {
            match self.transport.read_packet_timeout(IDLE_READ_TIMEOUT) {
                Ok(pkt) => {
                    self.consume_packet(&pkt);
                }
                Err(err) if err.kind() == io::ErrorKind::TimedOut => return Ok(self.latest),
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

    pub fn latest(&self) -> MonitorSnapshot {
        self.latest
    }

    pub fn close(self) {}

    fn consume_packet(&mut self, pkt: &Packet) -> bool {
        let Some(snapshot) = MonitorSnapshot::from_telemetry_packet(pkt, self.calibration) else {
            return false;
        };
        self.latest = snapshot;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::MonitorSnapshot;
    use rp2040_fred_protocol::bridge_proto::{MsgType, Packet};
    use rp2040_fred_protocol::dro_decode::Calibration;

    #[test]
    fn telemetry_packet_decodes_to_monitor_snapshot() {
        let packet = Packet::telemetry(9, 123, -100, 250, 780, 0x5A);
        let snapshot =
            MonitorSnapshot::from_telemetry_packet(&packet, Calibration::default()).expect("valid");

        assert_eq!(snapshot.tick, 123);
        assert_eq!(snapshot.x_counts, -100);
        assert_eq!(snapshot.z_counts, 250);
        assert_eq!(snapshot.spindle_rpm, 780);
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
