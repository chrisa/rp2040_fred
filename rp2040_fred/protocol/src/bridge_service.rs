#![allow(dead_code)]

use crate::bridge_proto::{MsgType, Packet};
use crate::dro_decode::{DroAssembler, DroSnapshot};
use crate::mock_bus::MockBusRunner;

pub const FLAG_ENABLED: u8 = 1 << 0;

pub struct BridgeService {
    telemetry_enabled: bool,
    telemetry_period_ms: u16,
    tick: u32,
    telemetry_seq: u16,
    bus_cycles: u32,
    tx_timeout_count: u32,
    rx_timeout_count: u32,
    mock: MockBusRunner,
    dro: DroAssembler,
}

impl BridgeService {
    pub const fn new() -> Self {
        Self {
            telemetry_enabled: false,
            telemetry_period_ms: 100,
            tick: 0,
            telemetry_seq: 1,
            bus_cycles: 0,
            tx_timeout_count: 0,
            rx_timeout_count: 0,
            mock: MockBusRunner::new(),
            dro: DroAssembler::new(),
        }
    }

    pub fn handle_request(&mut self, req: Packet, out: &mut [Packet; 2]) -> usize {
        match req.msg_type {
            MsgType::Ping => {
                out[0] = Packet::ack(req.seq, MsgType::Ping, 0);
                1
            }
            MsgType::TelemetrySet => {
                if req.payload_len < 1 {
                    out[0] = Packet::nack(req.seq, MsgType::TelemetrySet as u8, 1);
                    return 1;
                }
                self.telemetry_enabled = req.payload[0] != 0;
                if req.payload_len >= 3 {
                    self.telemetry_period_ms = u16::from_le_bytes([req.payload[1], req.payload[2]]);
                }
                out[0] = Packet::ack(req.seq, MsgType::TelemetrySet, 0);
                1
            }
            MsgType::SnapshotReq => {
                out[0] = Packet::telemetry(
                    req.seq,
                    self.tick,
                    self.snapshot().x_counts,
                    self.snapshot().z_counts,
                    self.snapshot().rpm,
                    self.flags(),
                );
                out[1] = Packet::ack(req.seq, MsgType::SnapshotReq, 0);
                2
            }
            _ => {
                out[0] = Packet::nack(req.seq, req.msg_type as u8, 0xFE);
                1
            }
        }
    }

    pub fn poll_telemetry_event(&mut self) -> Option<Packet> {
        if !self.telemetry_enabled {
            return None;
        }

        let frame = self.mock.step();
        self.tick = self.tick.wrapping_add(1);
        self.bus_cycles = self.bus_cycles.wrapping_add(1);
        self.dro.on_fc80_fcf1(frame.cmd_fc80, frame.response_fcf1);

        // Emit one telemetry packet per full DRO command cadence.
        if frame.cmd_fc80 != 0x0C {
            return None;
        }

        let s = self.snapshot();
        let pkt = Packet::telemetry(
            self.telemetry_seq,
            self.tick,
            s.x_counts,
            s.z_counts,
            s.rpm,
            self.flags(),
        );
        self.telemetry_seq = self.telemetry_seq.wrapping_add(1);
        Some(pkt)
    }

    pub fn health_packet(&mut self) -> Packet {
        let pkt = Packet::health(
            self.telemetry_seq,
            self.tx_timeout_count,
            self.rx_timeout_count,
            self.bus_cycles,
        );
        self.telemetry_seq = self.telemetry_seq.wrapping_add(1);
        pkt
    }

    pub fn telemetry_period_ms(&self) -> u16 {
        self.telemetry_period_ms
    }

    pub fn snapshot(&self) -> DroSnapshot {
        self.dro.snapshot()
    }

    fn flags(&self) -> u8 {
        if self.telemetry_enabled {
            FLAG_ENABLED
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bridge_proto::{MsgType, Packet};

    use super::BridgeService;

    #[test]
    fn ping_is_acked() {
        let mut svc = BridgeService::new();
        let mut out = [Packet::ping(0), Packet::ping(0)];
        let n = svc.handle_request(Packet::ping(7), &mut out);
        assert_eq!(n, 1);
        assert_eq!(out[0].msg_type, MsgType::Ack);
        assert_eq!(out[0].seq, 7);
    }

    #[test]
    fn telemetry_enable_changes_state_and_emits_events() {
        let mut svc = BridgeService::new();
        let mut out = [Packet::ping(0), Packet::ping(0)];
        let n = svc.handle_request(Packet::telemetry_set(9, true, 25), &mut out);
        assert_eq!(n, 1);
        assert_eq!(out[0].msg_type, MsgType::Ack);
        assert_eq!(svc.telemetry_period_ms(), 25);

        let mut seen = 0usize;
        for _ in 0..40 {
            if svc.poll_telemetry_event().is_some() {
                seen += 1;
            }
        }
        assert!(seen >= 2);
    }
}
