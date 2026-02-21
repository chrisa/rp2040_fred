#![allow(dead_code)]

pub const PACKET_MAGIC: u8 = 0xA5;
pub const PROTOCOL_VERSION: u8 = 1;
pub const PACKET_SIZE: usize = 32;
pub const PAYLOAD_SIZE: usize = 20;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgType {
    Ping = 0x01,
    TelemetrySet = 0x10,
    UnitCfg = 0x11,
    SnapshotReq = 0x12,
    CaptureSet = 0x13,
    Ack = 0x80,
    Nack = 0x81,
    Telemetry = 0x90,
    Health = 0x91,
    TraceSample = 0x92,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Ping),
            0x10 => Some(Self::TelemetrySet),
            0x11 => Some(Self::UnitCfg),
            0x12 => Some(Self::SnapshotReq),
            0x13 => Some(Self::CaptureSet),
            0x80 => Some(Self::Ack),
            0x81 => Some(Self::Nack),
            0x90 => Some(Self::Telemetry),
            0x91 => Some(Self::Health),
            0x92 => Some(Self::TraceSample),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    BadMagic,
    BadVersion,
    PayloadLen,
    UnknownMsgType,
    BadCrc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub msg_type: MsgType,
    pub seq: u16,
    pub payload_len: u8,
    pub payload: [u8; PAYLOAD_SIZE],
}

impl Packet {
    pub fn new(msg_type: MsgType, seq: u16, payload: &[u8]) -> Option<Self> {
        if payload.len() > PAYLOAD_SIZE {
            return None;
        }

        let mut fixed = [0u8; PAYLOAD_SIZE];
        fixed[..payload.len()].copy_from_slice(payload);
        Some(Self {
            msg_type,
            seq,
            payload_len: payload.len() as u8,
            payload: fixed,
        })
    }

    pub fn encode(&self) -> [u8; PACKET_SIZE] {
        let mut out = [0u8; PACKET_SIZE];
        out[0] = PACKET_MAGIC;
        out[1] = PROTOCOL_VERSION;
        out[2] = self.msg_type as u8;
        out[3] = self.payload_len;
        out[4..6].copy_from_slice(&self.seq.to_le_bytes());
        out[6..8].copy_from_slice(&0u16.to_le_bytes());
        out[8..32 - 4].copy_from_slice(&self.payload);
        let crc = crc32_ieee(&out[..28]);
        out[28..32].copy_from_slice(&crc.to_le_bytes());
        out
    }

    pub fn decode(raw: &[u8; PACKET_SIZE]) -> Result<Self, DecodeError> {
        if raw[0] != PACKET_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if raw[1] != PROTOCOL_VERSION {
            return Err(DecodeError::BadVersion);
        }
        let payload_len = raw[3];
        if payload_len as usize > PAYLOAD_SIZE {
            return Err(DecodeError::PayloadLen);
        }
        let msg_type = MsgType::from_u8(raw[2]).ok_or(DecodeError::UnknownMsgType)?;
        let expected_crc = u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]);
        let actual_crc = crc32_ieee(&raw[..28]);
        if expected_crc != actual_crc {
            return Err(DecodeError::BadCrc);
        }

        let seq = u16::from_le_bytes([raw[4], raw[5]]);
        let mut payload = [0u8; PAYLOAD_SIZE];
        payload.copy_from_slice(&raw[8..28]);
        Ok(Self {
            msg_type,
            seq,
            payload_len,
            payload,
        })
    }

    pub fn payload_used(&self) -> &[u8] {
        &self.payload[..self.payload_len as usize]
    }

    pub fn ping(seq: u16) -> Self {
        Self::new(MsgType::Ping, seq, &[]).expect("valid ping")
    }

    pub fn telemetry_set(seq: u16, enable: bool, period_ms: u16) -> Self {
        let payload = [enable as u8, period_ms as u8, (period_ms >> 8) as u8];
        Self::new(MsgType::TelemetrySet, seq, &payload).expect("valid telemetry_set")
    }

    pub fn capture_set(seq: u16, enable: bool) -> Self {
        let payload = [enable as u8];
        Self::new(MsgType::CaptureSet, seq, &payload).expect("valid capture_set")
    }

    pub fn ack(seq: u16, acked_type: MsgType, status: u8) -> Self {
        let payload = [acked_type as u8, status];
        Self::new(MsgType::Ack, seq, &payload).expect("valid ack")
    }

    pub fn nack(seq: u16, rejected_type: u8, reason: u8) -> Self {
        let payload = [rejected_type, reason];
        Self::new(MsgType::Nack, seq, &payload).expect("valid nack")
    }

    pub fn telemetry(
        seq: u16,
        tick: u32,
        x_counts: i32,
        z_counts: i32,
        rpm: u16,
        flags: u8,
    ) -> Self {
        let mut payload = [0u8; 16];
        payload[0..4].copy_from_slice(&tick.to_le_bytes());
        payload[4..8].copy_from_slice(&x_counts.to_le_bytes());
        payload[8..12].copy_from_slice(&z_counts.to_le_bytes());
        payload[12..14].copy_from_slice(&rpm.to_le_bytes());
        payload[14] = flags;
        payload[15] = 0;
        Self::new(MsgType::Telemetry, seq, &payload).expect("valid telemetry")
    }

    pub fn health(seq: u16, tx_timeout_count: u32, rx_timeout_count: u32, bus_cycles: u32) -> Self {
        let mut payload = [0u8; 12];
        payload[0..4].copy_from_slice(&tx_timeout_count.to_le_bytes());
        payload[4..8].copy_from_slice(&rx_timeout_count.to_le_bytes());
        payload[8..12].copy_from_slice(&bus_cycles.to_le_bytes());
        Self::new(MsgType::Health, seq, &payload).expect("valid health")
    }

    pub fn trace_sample(seq: u16, tick: u32, sample_bits: u32) -> Self {
        let mut payload = [0u8; 8];
        payload[0..4].copy_from_slice(&tick.to_le_bytes());
        payload[4..8].copy_from_slice(&sample_bits.to_le_bytes());
        Self::new(MsgType::TraceSample, seq, &payload).expect("valid trace sample")
    }
}

pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::{
        crc32_ieee, DecodeError, MsgType, Packet, PACKET_MAGIC, PACKET_SIZE, PROTOCOL_VERSION,
    };

    #[test]
    fn crc32_golden_vector() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn ping_roundtrip() {
        let pkt = Packet::ping(0x1234);
        let raw = pkt.encode();
        let got = Packet::decode(&raw).expect("decode");
        assert_eq!(got.msg_type, MsgType::Ping);
        assert_eq!(got.seq, 0x1234);
        assert_eq!(got.payload_len, 0);
    }

    #[test]
    fn telemetry_roundtrip() {
        let pkt = Packet::telemetry(5, 0x1122_3344, -12345, 54321, 1800, 0x03);
        let raw = pkt.encode();
        let got = Packet::decode(&raw).expect("decode");
        assert_eq!(got.msg_type, MsgType::Telemetry);
        assert_eq!(got.seq, 5);
        assert_eq!(got.payload_len, 16);

        let p = got.payload_used();
        assert_eq!(u32::from_le_bytes([p[0], p[1], p[2], p[3]]), 0x1122_3344);
        assert_eq!(i32::from_le_bytes([p[4], p[5], p[6], p[7]]), -12345);
        assert_eq!(i32::from_le_bytes([p[8], p[9], p[10], p[11]]), 54321);
        assert_eq!(u16::from_le_bytes([p[12], p[13]]), 1800);
        assert_eq!(p[14], 0x03);
    }

    #[test]
    fn capture_and_trace_roundtrip() {
        let capture = Packet::capture_set(0x22, true);
        let capture_raw = capture.encode();
        let capture_got = Packet::decode(&capture_raw).expect("decode capture");
        assert_eq!(capture_got.msg_type, MsgType::CaptureSet);
        assert_eq!(capture_got.seq, 0x22);
        assert_eq!(capture_got.payload_used(), &[1]);

        let trace = Packet::trace_sample(0x33, 0x0102_0304, 0xA5A5_5A5A);
        let trace_raw = trace.encode();
        let trace_got = Packet::decode(&trace_raw).expect("decode trace");
        assert_eq!(trace_got.msg_type, MsgType::TraceSample);
        assert_eq!(trace_got.seq, 0x33);
        assert_eq!(trace_got.payload_len, 8);
        let p = trace_got.payload_used();
        assert_eq!(u32::from_le_bytes([p[0], p[1], p[2], p[3]]), 0x0102_0304);
        assert_eq!(u32::from_le_bytes([p[4], p[5], p[6], p[7]]), 0xA5A5_5A5A);
    }

    #[test]
    fn decode_rejects_bad_crc() {
        let pkt = Packet::ack(7, MsgType::Ping, 0);
        let mut raw = pkt.encode();
        raw[10] ^= 0x55;
        assert_eq!(Packet::decode(&raw), Err(DecodeError::BadCrc));
    }

    #[test]
    fn decode_rejects_bad_header() {
        let mut raw = [0u8; PACKET_SIZE];
        raw[0] = PACKET_MAGIC;
        raw[1] = PROTOCOL_VERSION;
        raw[2] = 0x01;
        raw[3] = 0;
        let crc = crc32_ieee(&raw[..28]);
        raw[28..32].copy_from_slice(&crc.to_le_bytes());

        raw[0] = 0x00;
        assert_eq!(Packet::decode(&raw), Err(DecodeError::BadMagic));
    }
}
