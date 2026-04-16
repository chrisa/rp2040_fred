#![allow(dead_code)]

pub const PACKET_MAGIC: u8 = 0xA5;
pub const PROTOCOL_VERSION: u8 = 3;
pub const HEADER_SIZE: usize = 8;
pub const CRC_SIZE: usize = 4;
pub const PAYLOAD_SIZE: usize = 305;
pub const PACKET_SIZE: usize = HEADER_SIZE + PAYLOAD_SIZE + CRC_SIZE;
pub const MIN_PACKET_SIZE: usize = HEADER_SIZE + CRC_SIZE;
pub const TRACE_METADATA_SIZE: usize = 8;
pub const TRACE_PACKED_SAMPLE_SIZE: usize = 3;
pub const TRACE_SAMPLES_PER_PACKET: usize =
    (PAYLOAD_SIZE - TRACE_METADATA_SIZE) / TRACE_PACKED_SAMPLE_SIZE;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgType {
    Ping = 0x01,
    TelemetrySet = 0x10,
    UnitCfg = 0x11,
    SnapshotReq = 0x12,
    CaptureSet = 0x13,
    MockSet = 0x14,
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
            0x14 => Some(Self::MockSet),
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
    PacketLen,
    PayloadLen,
    UnknownMsgType,
    BadCrc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub msg_type: MsgType,
    pub seq: u16,
    pub payload_len: u16,
    pub payload: [u8; PAYLOAD_SIZE],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceSamples<'a> {
    pub dropped_samples_total: u32,
    pub rx_stall_count_total: u32,
    sample_bytes: &'a [u8],
}

impl<'a> TraceSamples<'a> {
    pub fn iter_samples(&self) -> impl Iterator<Item = u32> + 'a {
        self.sample_bytes
            .chunks_exact(TRACE_PACKED_SAMPLE_SIZE)
            .map(|chunk| unpack_trace_sample([chunk[0], chunk[1], chunk[2]]))
    }

    pub fn sample_count(&self) -> usize {
        self.sample_bytes.len() / TRACE_PACKED_SAMPLE_SIZE
    }

    pub fn packed_sample_bytes(&self) -> &'a [u8] {
        self.sample_bytes
    }
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
            payload_len: payload.len() as u16,
            payload: fixed,
        })
    }

    pub fn encode(&self) -> [u8; PACKET_SIZE] {
        let mut out = [0u8; PACKET_SIZE];
        out[0] = PACKET_MAGIC;
        out[1] = PROTOCOL_VERSION;
        out[2] = self.msg_type as u8;
        out[3] = 0;
        out[4..6].copy_from_slice(&self.seq.to_le_bytes());
        out[6..8].copy_from_slice(&self.payload_len.to_le_bytes());
        let encoded_len = self.encoded_len();
        let payload_len = self.payload_len as usize;
        out[HEADER_SIZE..HEADER_SIZE + payload_len].copy_from_slice(self.payload_used());
        let crc_offset = HEADER_SIZE + payload_len;
        let crc = crc32_ieee(&out[..crc_offset]);
        out[crc_offset..encoded_len].copy_from_slice(&crc.to_le_bytes());
        out
    }

    pub fn encoded_len(&self) -> usize {
        HEADER_SIZE + self.payload_len as usize + CRC_SIZE
    }

    pub fn decode(raw: &[u8]) -> Result<Self, DecodeError> {
        if raw.len() < MIN_PACKET_SIZE {
            return Err(DecodeError::PacketLen);
        }
        if raw[0] != PACKET_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if raw[1] != PROTOCOL_VERSION {
            return Err(DecodeError::BadVersion);
        }
        let payload_len = u16::from_le_bytes([raw[6], raw[7]]);
        if payload_len as usize > PAYLOAD_SIZE {
            return Err(DecodeError::PayloadLen);
        }
        let encoded_len = HEADER_SIZE + payload_len as usize + CRC_SIZE;
        if raw.len() != encoded_len {
            return Err(DecodeError::PacketLen);
        }
        let msg_type = MsgType::from_u8(raw[2]).ok_or(DecodeError::UnknownMsgType)?;
        let crc_offset = HEADER_SIZE + payload_len as usize;
        let expected_crc = u32::from_le_bytes([
            raw[crc_offset],
            raw[crc_offset + 1],
            raw[crc_offset + 2],
            raw[crc_offset + 3],
        ]);
        let actual_crc = crc32_ieee(&raw[..crc_offset]);
        if expected_crc != actual_crc {
            return Err(DecodeError::BadCrc);
        }

        let seq = u16::from_le_bytes([raw[4], raw[5]]);
        let mut payload = [0u8; PAYLOAD_SIZE];
        payload[..payload_len as usize].copy_from_slice(&raw[HEADER_SIZE..crc_offset]);
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

    pub fn mock_set(seq: u16, enable: bool) -> Self {
        let payload = [enable as u8];
        Self::new(MsgType::MockSet, seq, &payload).expect("valid mock_set")
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

    pub fn trace_samples(
        seq: u16,
        dropped_samples_total: u32,
        rx_stall_count_total: u32,
        samples: &[u32],
    ) -> Self {
        assert!(samples.len() <= TRACE_SAMPLES_PER_PACKET);

        let mut payload = [0u8; PAYLOAD_SIZE];
        payload[0..4].copy_from_slice(&dropped_samples_total.to_le_bytes());
        payload[4..8].copy_from_slice(&rx_stall_count_total.to_le_bytes());
        let mut used = TRACE_METADATA_SIZE;

        for sample in samples {
            let packed = pack_trace_sample(*sample);
            payload[used..used + TRACE_PACKED_SAMPLE_SIZE].copy_from_slice(&packed);
            used += TRACE_PACKED_SAMPLE_SIZE;
        }

        Self::new(MsgType::TraceSample, seq, &payload[..used]).expect("valid trace samples")
    }

    pub fn trace_sample(seq: u16, sample_bits: u32) -> Self {
        Self::trace_samples(seq, 0, 0, core::slice::from_ref(&sample_bits))
    }

    pub fn decode_trace_samples(&self) -> Option<TraceSamples<'_>> {
        if self.msg_type != MsgType::TraceSample
            || (self.payload_len as usize) < TRACE_METADATA_SIZE
        {
            return None;
        }

        let used = self.payload_used();
        let dropped_samples_total = u32::from_le_bytes([used[0], used[1], used[2], used[3]]);
        let rx_stall_count_total = u32::from_le_bytes([used[4], used[5], used[6], used[7]]);
        let sample_bytes = &used[TRACE_METADATA_SIZE..];
        if sample_bytes.len() % TRACE_PACKED_SAMPLE_SIZE != 0 {
            return None;
        }

        Some(TraceSamples {
            dropped_samples_total,
            rx_stall_count_total,
            sample_bytes,
        })
    }
}

pub fn pack_trace_sample(sample: u32) -> [u8; TRACE_PACKED_SAMPLE_SIZE] {
    [
        (sample & 0xFF) as u8,
        ((sample >> 8) & 0xFF) as u8,
        ((sample >> 16) & 0x01) as u8,
    ]
}

pub fn unpack_trace_sample(packed: [u8; TRACE_PACKED_SAMPLE_SIZE]) -> u32 {
    (packed[0] as u32)
        | ((packed[1] as u32) << 8)
        | (((packed[2] as u32) & 0x01) << 16)
        | (1 << 17)
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
        crc32_ieee, pack_trace_sample, unpack_trace_sample, DecodeError, MsgType, Packet, CRC_SIZE,
        HEADER_SIZE, MIN_PACKET_SIZE, PACKET_MAGIC, PROTOCOL_VERSION,
    };

    fn sample(data: u8, addr: u8, read: bool) -> u32 {
        (data as u32) | ((addr as u32) << 8) | ((read as u32) << 16) | (1 << 17)
    }

    #[test]
    fn crc32_golden_vector() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn ping_roundtrip() {
        let pkt = Packet::ping(0x1234);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode");
        assert_eq!(got.msg_type, MsgType::Ping);
        assert_eq!(got.seq, 0x1234);
        assert_eq!(got.payload_len, 0);
    }

    #[test]
    fn telemetry_roundtrip() {
        let pkt = Packet::telemetry(5, 0x1122_3344, -12345, 54321, 1800, 0x03);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode");
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
        let capture_got =
            Packet::decode(&capture_raw[..capture.encoded_len()]).expect("decode capture");
        assert_eq!(capture_got.msg_type, MsgType::CaptureSet);
        assert_eq!(capture_got.seq, 0x22);
        assert_eq!(capture_got.payload_used(), &[1]);

        let trace = Packet::trace_samples(
            0x33,
            7,
            2,
            &[sample(0x04, 0x03, false), sample(0x5A, 0xA5, true)],
        );
        let trace_raw = trace.encode();
        let trace_got = Packet::decode(&trace_raw[..trace.encoded_len()]).expect("decode trace");
        assert_eq!(trace_got.msg_type, MsgType::TraceSample);
        assert_eq!(trace_got.seq, 0x33);
        assert_eq!(trace_got.payload_len, 14);
        let trace_decoded = trace_got.decode_trace_samples().expect("trace payload");
        assert_eq!(trace_decoded.dropped_samples_total, 7);
        assert_eq!(trace_decoded.rx_stall_count_total, 2);
        assert_eq!(trace_decoded.sample_count(), 2);
        let mut samples = trace_decoded.iter_samples();
        assert_eq!(samples.next(), Some(sample(0x04, 0x03, false)));
        assert_eq!(samples.next(), Some(sample(0x5A, 0xA5, true)));
        assert_eq!(samples.next(), None);
    }

    #[test]
    fn decode_rejects_bad_crc() {
        let pkt = Packet::ack(7, MsgType::Ping, 0);
        let mut raw = pkt.encode();
        raw[10] ^= 0x55;
        assert_eq!(
            Packet::decode(&raw[..pkt.encoded_len()]),
            Err(DecodeError::BadCrc)
        );
    }

    #[test]
    fn packed_trace_sample_roundtrip() {
        let packed = pack_trace_sample(sample(0x34, 0xF1, true));
        assert_eq!(packed, [0x34, 0xF1, 0x01]);
        assert_eq!(unpack_trace_sample(packed), sample(0x34, 0xF1, true));
    }

    #[test]
    fn decode_rejects_bad_header() {
        let mut raw = [0u8; MIN_PACKET_SIZE];
        raw[0] = PACKET_MAGIC;
        raw[1] = PROTOCOL_VERSION;
        raw[2] = 0x01;
        raw[3] = 0;
        raw[4..6].copy_from_slice(&0u16.to_le_bytes());
        raw[6..8].copy_from_slice(&0u16.to_le_bytes());
        let crc = crc32_ieee(&raw[..HEADER_SIZE]);
        raw[HEADER_SIZE..HEADER_SIZE + CRC_SIZE].copy_from_slice(&crc.to_le_bytes());

        raw[0] = 0x00;
        assert_eq!(Packet::decode(&raw), Err(DecodeError::BadMagic));
    }
}
