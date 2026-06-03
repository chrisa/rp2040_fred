pub const PACKET_MAGIC: u8 = 0xA5;
pub const PROTOCOL_VERSION: u8 = 6;
pub const USB_VENDOR_CLASS: u8 = 0xFF;
pub const USB_VENDOR_SUBCLASS: u8 = 0x00;
pub const USB_PROTOCOL_MASTER: u8 = 0x01;
pub const USB_PROTOCOL_CAPTURE: u8 = 0x02;
pub const HEADER_SIZE: usize = 8;
pub const CRC_SIZE: usize = 4;
pub const PAYLOAD_SIZE: usize = 305;
pub const PACKET_SIZE: usize = HEADER_SIZE + PAYLOAD_SIZE + CRC_SIZE;
pub const MIN_PACKET_SIZE: usize = HEADER_SIZE + CRC_SIZE;
pub const TRACE_METADATA_SIZE: usize = 16;
pub const TRACE_PACKED_SAMPLE_SIZE: usize = 3;
pub const TRACE_SAMPLES_PER_PACKET: usize =
    (PAYLOAD_SIZE - TRACE_METADATA_SIZE) / TRACE_PACKED_SAMPLE_SIZE;
pub const TRACE_TIMESTAMP_UNKNOWN_US: u64 = u64::MAX;
pub const COMMAND_BLOCK_PAYLOAD_SIZE: usize = 20;
pub const COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE: usize = COMMAND_BLOCK_PAYLOAD_SIZE + 1;
pub const MAX_EXPERIMENT_SCRIPT_OPS: usize = 21;
pub const EXPERIMENT_BUS_OP_PAYLOAD_SIZE: usize = 12;
pub const EXPERIMENT_RUN_FIXED_PAYLOAD_SIZE: usize = COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE + 28;
pub const EXPERIMENT_RUN_MAX_PAYLOAD_SIZE: usize =
    EXPERIMENT_RUN_FIXED_PAYLOAD_SIZE + MAX_EXPERIMENT_SCRIPT_OPS * EXPERIMENT_BUS_OP_PAYLOAD_SIZE;
pub const COMMAND_BLOCK_FLAG_CYCLE_START_WAIT: u8 = 1 << 0;
pub const TELEMETRY_FLAG_ENABLED: u8 = 1 << 0;
pub const TELEMETRY_FLAG_CONTROLLER_BUSY: u8 = 1 << 1;
pub const TELEMETRY_FLAG_COMMAND_ACTIVE: u8 = 1 << 2;
pub const TELEMETRY_FLAG_CONTROLLER_ERROR: u8 = 1 << 3;
pub const EXPERIMENT_STATUS_ACTIVE: u8 = 1 << 0;
pub const EXPERIMENT_STATUS_DONE: u8 = 1 << 1;
pub const EXPERIMENT_STATUS_ERROR: u8 = 1 << 2;
pub const EXPERIMENT_STATUS_RECORDS_DROPPED: u8 = 1 << 3;
pub const EXPERIMENT_BUS_OP_STATUS_OK: u8 = 0;
pub const EXPERIMENT_BUS_OP_STATUS_TIMEOUT: u8 = 1;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgType {
    Ping = 0x01,
    TelemetrySet = 0x10,
    UnitCfg = 0x11,
    SnapshotReq = 0x12,
    CaptureSet = 0x13,
    CommandBlock = 0x14,
    ControllerAction = 0x15,
    ControllerStatusReq = 0x16,
    ExperimentRun = 0x17,
    ExperimentStatusReq = 0x18,
    Ack = 0x80,
    Nack = 0x81,
    Telemetry = 0x90,
    Health = 0x91,
    TraceSample = 0x92,
    ExperimentRecord = 0x93,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Ping),
            0x10 => Some(Self::TelemetrySet),
            0x11 => Some(Self::UnitCfg),
            0x12 => Some(Self::SnapshotReq),
            0x13 => Some(Self::CaptureSet),
            0x14 => Some(Self::CommandBlock),
            0x15 => Some(Self::ControllerAction),
            0x16 => Some(Self::ControllerStatusReq),
            0x17 => Some(Self::ExperimentRun),
            0x18 => Some(Self::ExperimentStatusReq),
            0x80 => Some(Self::Ack),
            0x81 => Some(Self::Nack),
            0x90 => Some(Self::Telemetry),
            0x91 => Some(Self::Health),
            0x92 => Some(Self::TraceSample),
            0x93 => Some(Self::ExperimentRecord),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerAction {
    CycleStartWait = 0x01,
}

impl ControllerAction {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::CycleStartWait),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpmServiceMode {
    Manual = 0x00,
    Remote = 0x01,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExperimentBusOpKind {
    #[default]
    DelayUs = 0x01,
    Read = 0x02,
    Write = 0x03,
    WriteGated = 0x04,
    ReadUntil = 0x05,
    PollFeedbackOnce = 0x06,
}

impl ExperimentBusOpKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::DelayUs),
            0x02 => Some(Self::Read),
            0x03 => Some(Self::Write),
            0x04 => Some(Self::WriteGated),
            0x05 => Some(Self::ReadUntil),
            0x06 => Some(Self::PollFeedbackOnce),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExperimentRecordKind {
    Sample = 0x01,
    BusOp = 0x02,
    Event = 0x03,
}

impl ExperimentRecordKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Sample),
            0x02 => Some(Self::BusOp),
            0x03 => Some(Self::Event),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExperimentEventKind {
    CommandLoaded = 0x01,
    CommandComplete = 0x02,
    Error = 0x03,
}

impl ExperimentEventKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::CommandLoaded),
            0x02 => Some(Self::CommandComplete),
            0x03 => Some(Self::Error),
            _ => None,
        }
    }
}

impl RpmServiceMode {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::Manual),
            0x01 => Some(Self::Remote),
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControllerStatus {
    pub flags: u8,
    pub pending_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExperimentStatus {
    pub flags: u8,
    pub pending_records: u32,
    pub dropped_records: u32,
    pub active_trial_id: u32,
}

impl ControllerStatus {
    pub fn is_idle(self) -> bool {
        self.pending_count == 0
            && (self.flags & (TELEMETRY_FLAG_CONTROLLER_BUSY | TELEMETRY_FLAG_COMMAND_ACTIVE)) == 0
    }

    pub fn has_error(self) -> bool {
        self.flags & TELEMETRY_FLAG_CONTROLLER_ERROR != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceSamples<'a> {
    pub timestamp_us: Option<u64>,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandBlock {
    pub m1: u8,
    pub m2: u8,
    pub m3: u16,
    pub m4: u16,
    pub m5: u16,
    pub m6: u16,
    pub m7: u16,
    pub m8: u16,
    pub m9: u16,
    pub m10: u32,
}

impl CommandBlock {
    pub fn to_payload(self) -> [u8; COMMAND_BLOCK_PAYLOAD_SIZE] {
        let mut payload = [0u8; COMMAND_BLOCK_PAYLOAD_SIZE];
        payload[0] = self.m1;
        payload[1] = self.m2;
        payload[2..4].copy_from_slice(&self.m3.to_le_bytes());
        payload[4..6].copy_from_slice(&self.m4.to_le_bytes());
        payload[6..8].copy_from_slice(&self.m5.to_le_bytes());
        payload[8..10].copy_from_slice(&self.m6.to_le_bytes());
        payload[10..12].copy_from_slice(&self.m7.to_le_bytes());
        payload[12..14].copy_from_slice(&self.m8.to_le_bytes());
        payload[14..16].copy_from_slice(&self.m9.to_le_bytes());
        payload[16..20].copy_from_slice(&self.m10.to_le_bytes());
        payload
    }

    pub fn from_payload(payload: &[u8]) -> Option<Self> {
        if payload.len() != COMMAND_BLOCK_PAYLOAD_SIZE {
            return None;
        }

        Some(Self {
            m1: payload[0],
            m2: payload[1],
            m3: u16::from_le_bytes([payload[2], payload[3]]),
            m4: u16::from_le_bytes([payload[4], payload[5]]),
            m5: u16::from_le_bytes([payload[6], payload[7]]),
            m6: u16::from_le_bytes([payload[8], payload[9]]),
            m7: u16::from_le_bytes([payload[10], payload[11]]),
            m8: u16::from_le_bytes([payload[12], payload[13]]),
            m9: u16::from_le_bytes([payload[14], payload[15]]),
            m10: u32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandBlockRequest {
    pub block: CommandBlock,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExperimentBusOp {
    pub kind: ExperimentBusOpKind,
    pub addr: u8,
    pub value: u8,
    pub mask: u8,
    pub match_value: u8,
    pub arg_us: u32,
}

impl ExperimentBusOp {
    pub fn delay_us(delay_us: u32) -> Self {
        Self {
            kind: ExperimentBusOpKind::DelayUs,
            arg_us: delay_us,
            ..Self::default()
        }
    }

    pub fn read(addr: u8) -> Self {
        Self {
            kind: ExperimentBusOpKind::Read,
            addr,
            ..Self::default()
        }
    }

    pub fn write(addr: u8, value: u8) -> Self {
        Self {
            kind: ExperimentBusOpKind::Write,
            addr,
            value,
            ..Self::default()
        }
    }

    pub fn write_gated(addr: u8, value: u8) -> Self {
        Self {
            kind: ExperimentBusOpKind::WriteGated,
            addr,
            value,
            ..Self::default()
        }
    }

    pub fn read_until(addr: u8, mask: u8, match_value: u8, timeout_us: u32) -> Self {
        Self {
            kind: ExperimentBusOpKind::ReadUntil,
            addr,
            mask,
            match_value,
            arg_us: timeout_us,
            ..Self::default()
        }
    }

    pub fn poll_feedback_once() -> Self {
        Self {
            kind: ExperimentBusOpKind::PollFeedbackOnce,
            ..Self::default()
        }
    }

    pub fn to_payload(self) -> [u8; EXPERIMENT_BUS_OP_PAYLOAD_SIZE] {
        let mut payload = [0u8; EXPERIMENT_BUS_OP_PAYLOAD_SIZE];
        payload[0] = self.kind as u8;
        payload[1] = self.addr;
        payload[2] = self.value;
        payload[3] = self.mask;
        payload[4] = self.match_value;
        payload[8..12].copy_from_slice(&self.arg_us.to_le_bytes());
        payload
    }

    pub fn from_payload(payload: &[u8]) -> Option<Self> {
        if payload.len() != EXPERIMENT_BUS_OP_PAYLOAD_SIZE {
            return None;
        }

        Some(Self {
            kind: ExperimentBusOpKind::from_u8(payload[0])?,
            addr: payload[1],
            value: payload[2],
            mask: payload[3],
            match_value: payload[4],
            arg_us: u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExperimentRunRequest {
    pub command: CommandBlockRequest,
    pub trial_id: u32,
    pub x_counts: i32,
    pub z_counts: i32,
    pub feed: u32,
    pub slew: u16,
    pub feedback_period_ms: u16,
    pub flags: u8,
    pub script_len: u8,
    pub script: [ExperimentBusOp; MAX_EXPERIMENT_SCRIPT_OPS],
}

impl Default for ExperimentRunRequest {
    fn default() -> Self {
        Self {
            command: CommandBlockRequest::default(),
            trial_id: 0,
            x_counts: 0,
            z_counts: 0,
            feed: 0,
            slew: 0,
            feedback_period_ms: 10,
            flags: 0,
            script_len: 0,
            script: [ExperimentBusOp::default(); MAX_EXPERIMENT_SCRIPT_OPS],
        }
    }
}

impl ExperimentRunRequest {
    pub fn to_payload(self) -> [u8; EXPERIMENT_RUN_MAX_PAYLOAD_SIZE] {
        let mut payload = [0u8; EXPERIMENT_RUN_MAX_PAYLOAD_SIZE];
        payload[..COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE].copy_from_slice(&self.command.to_payload());
        let mut offset = COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE;
        payload[offset..offset + 4].copy_from_slice(&self.trial_id.to_le_bytes());
        offset += 4;
        payload[offset..offset + 4].copy_from_slice(&self.x_counts.to_le_bytes());
        offset += 4;
        payload[offset..offset + 4].copy_from_slice(&self.z_counts.to_le_bytes());
        offset += 4;
        payload[offset..offset + 4].copy_from_slice(&self.feed.to_le_bytes());
        offset += 4;
        payload[offset..offset + 2].copy_from_slice(&self.slew.to_le_bytes());
        offset += 2;
        payload[offset..offset + 2].copy_from_slice(&self.feedback_period_ms.to_le_bytes());
        offset += 2;
        payload[offset] = self.flags;
        offset += 1;
        payload[offset] = self.script_len;
        offset += 1;
        payload[offset..offset + 6].copy_from_slice(&[0; 6]);
        offset += 6;

        for op in self.script.iter().take(self.script_len as usize) {
            payload[offset..offset + EXPERIMENT_BUS_OP_PAYLOAD_SIZE]
                .copy_from_slice(&op.to_payload());
            offset += EXPERIMENT_BUS_OP_PAYLOAD_SIZE;
        }

        payload
    }

    pub fn payload_len(self) -> usize {
        EXPERIMENT_RUN_FIXED_PAYLOAD_SIZE
            + self.script_len as usize * EXPERIMENT_BUS_OP_PAYLOAD_SIZE
    }

    pub fn from_payload(payload: &[u8]) -> Option<Self> {
        if payload.len() < EXPERIMENT_RUN_FIXED_PAYLOAD_SIZE {
            return None;
        }

        let command =
            CommandBlockRequest::from_payload(&payload[..COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE])?;
        let mut offset = COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE;
        let trial_id = u32::from_le_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]);
        offset += 4;
        let x_counts = i32::from_le_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]);
        offset += 4;
        let z_counts = i32::from_le_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]);
        offset += 4;
        let feed = u32::from_le_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]);
        offset += 4;
        let slew = u16::from_le_bytes([payload[offset], payload[offset + 1]]);
        offset += 2;
        let feedback_period_ms = u16::from_le_bytes([payload[offset], payload[offset + 1]]);
        offset += 2;
        let flags = payload[offset];
        offset += 1;
        let script_len = payload[offset];
        offset += 1;
        offset += 6;

        if script_len as usize > MAX_EXPERIMENT_SCRIPT_OPS {
            return None;
        }
        let expected_len = EXPERIMENT_RUN_FIXED_PAYLOAD_SIZE
            + script_len as usize * EXPERIMENT_BUS_OP_PAYLOAD_SIZE;
        if payload.len() != expected_len {
            return None;
        }

        let mut script = [ExperimentBusOp::default(); MAX_EXPERIMENT_SCRIPT_OPS];
        for op in script.iter_mut().take(script_len as usize) {
            *op = ExperimentBusOp::from_payload(
                &payload[offset..offset + EXPERIMENT_BUS_OP_PAYLOAD_SIZE],
            )?;
            offset += EXPERIMENT_BUS_OP_PAYLOAD_SIZE;
        }

        Some(Self {
            command,
            trial_id,
            x_counts,
            z_counts,
            feed,
            slew,
            feedback_period_ms,
            flags,
            script_len,
            script,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExperimentSampleRecord {
    pub trial_id: u32,
    pub timestamp_us: u64,
    pub sample_index: u64,
    pub x_counts: i32,
    pub z_counts: i32,
    pub rpm: u16,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExperimentBusOpRecord {
    pub trial_id: u32,
    pub timestamp_us: u64,
    pub op_index: u8,
    pub op_kind: ExperimentBusOpKind,
    pub status: u8,
    pub addr: u8,
    pub write_value: u8,
    pub read_value: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExperimentEventRecord {
    pub trial_id: u32,
    pub timestamp_us: u64,
    pub event: ExperimentEventKind,
    pub status: u8,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExperimentRecord {
    Sample(ExperimentSampleRecord),
    BusOp(ExperimentBusOpRecord),
    Event(ExperimentEventRecord),
}

impl CommandBlockRequest {
    pub fn to_payload(self) -> [u8; COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE] {
        let mut payload = [0u8; COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE];
        payload[..COMMAND_BLOCK_PAYLOAD_SIZE].copy_from_slice(&self.block.to_payload());
        payload[COMMAND_BLOCK_PAYLOAD_SIZE] = self.flags;
        payload
    }

    pub fn from_payload(payload: &[u8]) -> Option<Self> {
        match payload.len() {
            COMMAND_BLOCK_PAYLOAD_SIZE => Some(Self {
                block: CommandBlock::from_payload(payload)?,
                flags: 0,
            }),
            COMMAND_BLOCK_REQUEST_PAYLOAD_SIZE => Some(Self {
                block: CommandBlock::from_payload(&payload[..COMMAND_BLOCK_PAYLOAD_SIZE])?,
                flags: payload[COMMAND_BLOCK_PAYLOAD_SIZE],
            }),
            _ => None,
        }
    }

    pub fn cycle_start_wait(self) -> bool {
        self.flags & COMMAND_BLOCK_FLAG_CYCLE_START_WAIT != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerActionRequest {
    pub action: ControllerAction,
}

impl ControllerActionRequest {
    pub fn from_payload(payload: &[u8]) -> Option<Self> {
        match payload {
            [action] => Some(Self {
                action: ControllerAction::from_u8(*action)?,
            }),
            _ => None,
        }
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

    pub fn telemetry_set(
        seq: u16,
        enable: bool,
        period_ms: u16,
        rpm_service_mode: RpmServiceMode,
    ) -> Self {
        let payload = [
            enable as u8,
            period_ms as u8,
            (period_ms >> 8) as u8,
            rpm_service_mode as u8,
        ];
        Self::new(MsgType::TelemetrySet, seq, &payload).expect("valid telemetry_set")
    }

    pub fn capture_set(seq: u16, enable: bool) -> Self {
        let payload = [enable as u8];
        Self::new(MsgType::CaptureSet, seq, &payload).expect("valid capture_set")
    }

    pub fn command_block(seq: u16, block: CommandBlock) -> Self {
        Self::new(MsgType::CommandBlock, seq, &block.to_payload()).expect("valid command_block")
    }

    pub fn command_block_request(seq: u16, request: CommandBlockRequest) -> Self {
        Self::new(MsgType::CommandBlock, seq, &request.to_payload())
            .expect("valid command_block request")
    }

    pub fn command_block_with_flags(seq: u16, block: CommandBlock, flags: u8) -> Self {
        Self::command_block_request(seq, CommandBlockRequest { block, flags })
    }

    pub fn controller_action(seq: u16, action: ControllerAction) -> Self {
        Self::new(MsgType::ControllerAction, seq, &[action as u8]).expect("valid controller_action")
    }

    pub fn controller_status_req(seq: u16) -> Self {
        Self::new(MsgType::ControllerStatusReq, seq, &[]).expect("valid controller_status_req")
    }

    pub fn controller_status_ack(seq: u16, status: ControllerStatus) -> Self {
        let mut payload = [0u8; 7];
        payload[0] = MsgType::ControllerStatusReq as u8;
        payload[1] = 0;
        payload[2] = status.flags;
        payload[3..7].copy_from_slice(&status.pending_count.to_le_bytes());
        Self::new(MsgType::Ack, seq, &payload).expect("valid controller_status_ack")
    }

    pub fn experiment_run(seq: u16, request: ExperimentRunRequest) -> Self {
        let payload = request.to_payload();
        Self::new(
            MsgType::ExperimentRun,
            seq,
            &payload[..request.payload_len()],
        )
        .expect("valid experiment_run")
    }

    pub fn experiment_status_req(seq: u16) -> Self {
        Self::new(MsgType::ExperimentStatusReq, seq, &[]).expect("valid experiment_status_req")
    }

    pub fn experiment_status_ack(seq: u16, status: ExperimentStatus) -> Self {
        let mut payload = [0u8; 15];
        payload[0] = MsgType::ExperimentStatusReq as u8;
        payload[1] = 0;
        payload[2] = status.flags;
        payload[3..7].copy_from_slice(&status.pending_records.to_le_bytes());
        payload[7..11].copy_from_slice(&status.dropped_records.to_le_bytes());
        payload[11..15].copy_from_slice(&status.active_trial_id.to_le_bytes());
        Self::new(MsgType::Ack, seq, &payload).expect("valid experiment_status_ack")
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
        timestamp_us: Option<u64>,
        dropped_samples_total: u32,
        rx_stall_count_total: u32,
        samples: &[u32],
    ) -> Self {
        assert!(samples.len() <= TRACE_SAMPLES_PER_PACKET);

        let mut payload = [0u8; PAYLOAD_SIZE];
        payload[0..8].copy_from_slice(
            &timestamp_us
                .unwrap_or(TRACE_TIMESTAMP_UNKNOWN_US)
                .to_le_bytes(),
        );
        payload[8..12].copy_from_slice(&dropped_samples_total.to_le_bytes());
        payload[12..16].copy_from_slice(&rx_stall_count_total.to_le_bytes());
        let mut used = TRACE_METADATA_SIZE;

        for sample in samples {
            let packed = pack_trace_sample(*sample);
            payload[used..used + TRACE_PACKED_SAMPLE_SIZE].copy_from_slice(&packed);
            used += TRACE_PACKED_SAMPLE_SIZE;
        }

        Self::new(MsgType::TraceSample, seq, &payload[..used]).expect("valid trace samples")
    }

    pub fn trace_sample(seq: u16, sample_bits: u32) -> Self {
        Self::trace_samples(seq, None, 0, 0, core::slice::from_ref(&sample_bits))
    }

    pub fn experiment_sample(seq: u16, record: ExperimentSampleRecord) -> Self {
        let mut payload = [0u8; 32];
        payload[0] = ExperimentRecordKind::Sample as u8;
        payload[1] = record.flags;
        payload[2..4].copy_from_slice(&record.rpm.to_le_bytes());
        payload[4..8].copy_from_slice(&record.trial_id.to_le_bytes());
        payload[8..16].copy_from_slice(&record.timestamp_us.to_le_bytes());
        payload[16..24].copy_from_slice(&record.sample_index.to_le_bytes());
        payload[24..28].copy_from_slice(&record.x_counts.to_le_bytes());
        payload[28..32].copy_from_slice(&record.z_counts.to_le_bytes());
        Self::new(MsgType::ExperimentRecord, seq, &payload).expect("valid experiment_sample")
    }

    pub fn experiment_bus_op(seq: u16, record: ExperimentBusOpRecord) -> Self {
        let mut payload = [0u8; 24];
        payload[0] = ExperimentRecordKind::BusOp as u8;
        payload[1] = record.op_index;
        payload[2] = record.op_kind as u8;
        payload[3] = record.status;
        payload[4..8].copy_from_slice(&record.trial_id.to_le_bytes());
        payload[8..16].copy_from_slice(&record.timestamp_us.to_le_bytes());
        payload[16] = record.addr;
        payload[17] = record.write_value;
        payload[18] = record.read_value;
        Self::new(MsgType::ExperimentRecord, seq, &payload).expect("valid experiment_bus_op")
    }

    pub fn experiment_event(seq: u16, record: ExperimentEventRecord) -> Self {
        let mut payload = [0u8; 20];
        payload[0] = ExperimentRecordKind::Event as u8;
        payload[1] = record.event as u8;
        payload[2] = record.status;
        payload[3] = record.flags;
        payload[4..8].copy_from_slice(&record.trial_id.to_le_bytes());
        payload[8..16].copy_from_slice(&record.timestamp_us.to_le_bytes());
        Self::new(MsgType::ExperimentRecord, seq, &payload).expect("valid experiment_event")
    }

    pub fn decode_trace_samples(&self) -> Option<TraceSamples<'_>> {
        if self.msg_type != MsgType::TraceSample
            || (self.payload_len as usize) < TRACE_METADATA_SIZE
        {
            return None;
        }

        let used = self.payload_used();
        let raw_timestamp_us = u64::from_le_bytes([
            used[0], used[1], used[2], used[3], used[4], used[5], used[6], used[7],
        ]);
        let timestamp_us =
            (raw_timestamp_us != TRACE_TIMESTAMP_UNKNOWN_US).then_some(raw_timestamp_us);
        let dropped_samples_total = u32::from_le_bytes([used[8], used[9], used[10], used[11]]);
        let rx_stall_count_total = u32::from_le_bytes([used[12], used[13], used[14], used[15]]);
        let sample_bytes = &used[TRACE_METADATA_SIZE..];
        if !sample_bytes.len().is_multiple_of(TRACE_PACKED_SAMPLE_SIZE) {
            return None;
        }

        Some(TraceSamples {
            timestamp_us,
            dropped_samples_total,
            rx_stall_count_total,
            sample_bytes,
        })
    }

    pub fn decode_command_block(&self) -> Option<CommandBlock> {
        if self.msg_type != MsgType::CommandBlock {
            return None;
        }
        CommandBlock::from_payload(self.payload_used())
    }

    pub fn decode_command_block_request(&self) -> Option<CommandBlockRequest> {
        if self.msg_type != MsgType::CommandBlock {
            return None;
        }
        CommandBlockRequest::from_payload(self.payload_used())
    }

    pub fn decode_experiment_run(&self) -> Option<ExperimentRunRequest> {
        if self.msg_type != MsgType::ExperimentRun {
            return None;
        }
        ExperimentRunRequest::from_payload(self.payload_used())
    }

    pub fn decode_controller_action(&self) -> Option<ControllerAction> {
        self.decode_controller_action_request()
            .map(|request| request.action)
    }

    pub fn decode_controller_action_request(&self) -> Option<ControllerActionRequest> {
        if self.msg_type != MsgType::ControllerAction {
            return None;
        }
        ControllerActionRequest::from_payload(self.payload_used())
    }

    pub fn decode_controller_status_ack(&self) -> Option<ControllerStatus> {
        if self.msg_type != MsgType::Ack
            || self.payload_len < 7
            || self.payload[0] != MsgType::ControllerStatusReq as u8
            || self.payload[1] != 0
        {
            return None;
        }

        Some(ControllerStatus {
            flags: self.payload[2],
            pending_count: u32::from_le_bytes([
                self.payload[3],
                self.payload[4],
                self.payload[5],
                self.payload[6],
            ]),
        })
    }

    pub fn decode_experiment_status_ack(&self) -> Option<ExperimentStatus> {
        if self.msg_type != MsgType::Ack
            || self.payload_len < 15
            || self.payload[0] != MsgType::ExperimentStatusReq as u8
            || self.payload[1] != 0
        {
            return None;
        }

        Some(ExperimentStatus {
            flags: self.payload[2],
            pending_records: u32::from_le_bytes([
                self.payload[3],
                self.payload[4],
                self.payload[5],
                self.payload[6],
            ]),
            dropped_records: u32::from_le_bytes([
                self.payload[7],
                self.payload[8],
                self.payload[9],
                self.payload[10],
            ]),
            active_trial_id: u32::from_le_bytes([
                self.payload[11],
                self.payload[12],
                self.payload[13],
                self.payload[14],
            ]),
        })
    }

    pub fn decode_experiment_record(&self) -> Option<ExperimentRecord> {
        if self.msg_type != MsgType::ExperimentRecord || self.payload_len < 1 {
            return None;
        }

        match ExperimentRecordKind::from_u8(self.payload[0])? {
            ExperimentRecordKind::Sample => {
                if self.payload_len < 32 {
                    return None;
                }
                Some(ExperimentRecord::Sample(ExperimentSampleRecord {
                    trial_id: u32::from_le_bytes([
                        self.payload[4],
                        self.payload[5],
                        self.payload[6],
                        self.payload[7],
                    ]),
                    timestamp_us: u64::from_le_bytes([
                        self.payload[8],
                        self.payload[9],
                        self.payload[10],
                        self.payload[11],
                        self.payload[12],
                        self.payload[13],
                        self.payload[14],
                        self.payload[15],
                    ]),
                    sample_index: u64::from_le_bytes([
                        self.payload[16],
                        self.payload[17],
                        self.payload[18],
                        self.payload[19],
                        self.payload[20],
                        self.payload[21],
                        self.payload[22],
                        self.payload[23],
                    ]),
                    x_counts: i32::from_le_bytes([
                        self.payload[24],
                        self.payload[25],
                        self.payload[26],
                        self.payload[27],
                    ]),
                    z_counts: i32::from_le_bytes([
                        self.payload[28],
                        self.payload[29],
                        self.payload[30],
                        self.payload[31],
                    ]),
                    rpm: u16::from_le_bytes([self.payload[2], self.payload[3]]),
                    flags: self.payload[1],
                }))
            }
            ExperimentRecordKind::BusOp => {
                if self.payload_len < 24 {
                    return None;
                }
                Some(ExperimentRecord::BusOp(ExperimentBusOpRecord {
                    trial_id: u32::from_le_bytes([
                        self.payload[4],
                        self.payload[5],
                        self.payload[6],
                        self.payload[7],
                    ]),
                    timestamp_us: u64::from_le_bytes([
                        self.payload[8],
                        self.payload[9],
                        self.payload[10],
                        self.payload[11],
                        self.payload[12],
                        self.payload[13],
                        self.payload[14],
                        self.payload[15],
                    ]),
                    op_index: self.payload[1],
                    op_kind: ExperimentBusOpKind::from_u8(self.payload[2])?,
                    status: self.payload[3],
                    addr: self.payload[16],
                    write_value: self.payload[17],
                    read_value: self.payload[18],
                }))
            }
            ExperimentRecordKind::Event => {
                if self.payload_len < 20 {
                    return None;
                }
                Some(ExperimentRecord::Event(ExperimentEventRecord {
                    trial_id: u32::from_le_bytes([
                        self.payload[4],
                        self.payload[5],
                        self.payload[6],
                        self.payload[7],
                    ]),
                    timestamp_us: u64::from_le_bytes([
                        self.payload[8],
                        self.payload[9],
                        self.payload[10],
                        self.payload[11],
                        self.payload[12],
                        self.payload[13],
                        self.payload[14],
                        self.payload[15],
                    ]),
                    event: ExperimentEventKind::from_u8(self.payload[1])?,
                    status: self.payload[2],
                    flags: self.payload[3],
                }))
            }
        }
    }
}

pub fn pack_trace_sample(sample: u32) -> [u8; TRACE_PACKED_SAMPLE_SIZE] {
    [
        (sample & 0xFF) as u8,
        ((sample >> 8) & 0xFF) as u8,
        ((sample >> 16) & 0xFF) as u8,
    ]
}

pub fn unpack_trace_sample(packed: [u8; TRACE_PACKED_SAMPLE_SIZE]) -> u32 {
    (packed[0] as u32) | ((packed[1] as u32) << 8) | ((packed[2] as u32) << 16)
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
        crc32_ieee, pack_trace_sample, unpack_trace_sample, CommandBlock, CommandBlockRequest,
        ControllerAction, ControllerStatus, DecodeError, ExperimentBusOp, ExperimentBusOpKind,
        ExperimentBusOpRecord, ExperimentEventKind, ExperimentEventRecord, ExperimentRecord,
        ExperimentRunRequest, ExperimentSampleRecord, ExperimentStatus, MsgType, Packet,
        RpmServiceMode, COMMAND_BLOCK_FLAG_CYCLE_START_WAIT, CRC_SIZE, EXPERIMENT_STATUS_ACTIVE,
        EXPERIMENT_STATUS_DONE, HEADER_SIZE, MIN_PACKET_SIZE, PACKET_MAGIC, PROTOCOL_VERSION,
        TELEMETRY_FLAG_COMMAND_ACTIVE, TELEMETRY_FLAG_CONTROLLER_BUSY,
        TELEMETRY_FLAG_CONTROLLER_ERROR,
    };

    fn sample(data: u8, addr: u8, read: bool) -> u32 {
        (data as u32) | ((addr as u32) << 8) | ((read as u32) << 16) | (1 << 17)
    }

    fn raw_word(value: i16) -> u16 {
        u16::from_le_bytes(value.to_le_bytes())
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
    fn telemetry_set_carries_rpm_service_mode() {
        let pkt = Packet::telemetry_set(9, true, 25, RpmServiceMode::Remote);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode");

        assert_eq!(got.msg_type, MsgType::TelemetrySet);
        assert_eq!(got.payload_len, 4);
        assert_eq!(
            got.payload_used(),
            &[1, 25, 0, RpmServiceMode::Remote as u8]
        );
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
            Some(123_456),
            7,
            2,
            &[sample(0x04, 0x03, false), sample(0x5A, 0xA5, true)],
        );
        let trace_raw = trace.encode();
        let trace_got = Packet::decode(&trace_raw[..trace.encoded_len()]).expect("decode trace");
        assert_eq!(trace_got.msg_type, MsgType::TraceSample);
        assert_eq!(trace_got.seq, 0x33);
        assert_eq!(trace_got.payload_len, 22);
        let trace_decoded = trace_got.decode_trace_samples().expect("trace payload");
        assert_eq!(trace_decoded.timestamp_us, Some(123_456));
        assert_eq!(trace_decoded.dropped_samples_total, 7);
        assert_eq!(trace_decoded.rx_stall_count_total, 2);
        assert_eq!(trace_decoded.sample_count(), 2);
        let mut samples = trace_decoded.iter_samples();
        assert_eq!(samples.next(), Some(sample(0x04, 0x03, false)));
        assert_eq!(samples.next(), Some(sample(0x5A, 0xA5, true)));
        assert_eq!(samples.next(), None);
    }

    #[test]
    fn command_block_roundtrip() {
        let block = CommandBlock {
            m1: 1,
            m2: 0,
            m3: raw_word(-126),
            m4: raw_word(1500),
            m5: raw_word(-832),
            m6: 0x1234,
            m7: 0,
            m8: 400,
            m9: 61,
            m10: 0x0001_7FFF,
        };

        let pkt = Packet::command_block(0x44, block);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode command block");

        assert_eq!(got.msg_type, MsgType::CommandBlock);
        assert_eq!(got.seq, 0x44);
        assert_eq!(got.payload_len, 20);
        assert_eq!(got.decode_command_block(), Some(block));
        assert_eq!(
            got.decode_command_block_request(),
            Some(CommandBlockRequest { block, flags: 0 })
        );
    }

    #[test]
    fn command_block_request_roundtrip() {
        let request = CommandBlockRequest {
            block: CommandBlock {
                m1: 84,
                m2: 0,
                m3: raw_word(-126),
                m4: raw_word(1500),
                m5: 0,
                m6: 0,
                m7: 0,
                m8: 0,
                m9: 61,
                m10: 0x0001_7fff,
            },
            flags: COMMAND_BLOCK_FLAG_CYCLE_START_WAIT,
        };

        let pkt = Packet::command_block_request(0x46, request);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode command request");

        assert_eq!(got.msg_type, MsgType::CommandBlock);
        assert_eq!(got.seq, 0x46);
        assert_eq!(got.payload_len, 21);
        assert_eq!(got.decode_command_block_request(), Some(request));
        assert!(request.cycle_start_wait());
    }

    #[test]
    fn short_command_block_payload_is_rejected() {
        let pkt = Packet::new(MsgType::CommandBlock, 0x45, &[1, 2]).expect("packet");
        assert_eq!(pkt.decode_command_block(), None);
        assert_eq!(pkt.decode_command_block_request(), None);
    }

    #[test]
    fn controller_action_roundtrip() {
        let pkt = Packet::controller_action(0x47, ControllerAction::CycleStartWait);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode controller action");

        assert_eq!(got.msg_type, MsgType::ControllerAction);
        assert_eq!(got.seq, 0x47);
        assert_eq!(
            got.decode_controller_action(),
            Some(ControllerAction::CycleStartWait)
        );
        let request = got
            .decode_controller_action_request()
            .expect("controller action request");
        assert_eq!(request.action, ControllerAction::CycleStartWait);
    }

    #[test]
    fn controller_status_ack_roundtrip() {
        let status = ControllerStatus {
            flags: TELEMETRY_FLAG_CONTROLLER_BUSY
                | TELEMETRY_FLAG_COMMAND_ACTIVE
                | TELEMETRY_FLAG_CONTROLLER_ERROR,
            pending_count: 3,
        };
        let pkt = Packet::controller_status_ack(0x48, status);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode controller status");

        assert_eq!(got.msg_type, MsgType::Ack);
        assert_eq!(got.seq, 0x48);
        assert_eq!(got.decode_controller_status_ack(), Some(status));
        assert!(!status.is_idle());
        assert!(status.has_error());
    }

    #[test]
    fn experiment_run_roundtrip() {
        let mut request = ExperimentRunRequest {
            command: CommandBlockRequest {
                block: CommandBlock {
                    m1: 1,
                    m2: 0,
                    m3: raw_word(-10),
                    m4: raw_word(250),
                    m5: 0,
                    m6: 0,
                    m7: 0,
                    m8: 950,
                    m9: 61,
                    m10: 0,
                },
                flags: 0,
            },
            trial_id: 42,
            x_counts: -20,
            z_counts: 250,
            feed: 100,
            slew: 61,
            feedback_period_ms: 10,
            flags: 0xA0,
            script_len: 3,
            ..ExperimentRunRequest::default()
        };
        request.script[0] = ExperimentBusOp::delay_us(50_000);
        request.script[1] = ExperimentBusOp::write_gated(0xAF, 0);
        request.script[2] = ExperimentBusOp::read_until(0xF0, 0x80, 0, 10_000);

        let pkt = Packet::experiment_run(0x49, request);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode experiment");

        assert_eq!(got.msg_type, MsgType::ExperimentRun);
        assert_eq!(got.seq, 0x49);
        assert_eq!(got.decode_experiment_run(), Some(request));
    }

    #[test]
    fn experiment_status_ack_roundtrip() {
        let status = ExperimentStatus {
            flags: EXPERIMENT_STATUS_ACTIVE | EXPERIMENT_STATUS_DONE,
            pending_records: 12,
            dropped_records: 3,
            active_trial_id: 99,
        };
        let pkt = Packet::experiment_status_ack(0x4A, status);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode experiment status");

        assert_eq!(got.msg_type, MsgType::Ack);
        assert_eq!(got.decode_experiment_status_ack(), Some(status));
    }

    #[test]
    fn experiment_records_roundtrip() {
        let sample = ExperimentSampleRecord {
            trial_id: 7,
            timestamp_us: 123_456,
            sample_index: 9,
            x_counts: -12,
            z_counts: 345,
            rpm: 780,
            flags: 0x5A,
        };
        let pkt = Packet::experiment_sample(0x4B, sample);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode sample");
        assert_eq!(
            got.decode_experiment_record(),
            Some(ExperimentRecord::Sample(sample))
        );

        let bus_op = ExperimentBusOpRecord {
            trial_id: 7,
            timestamp_us: 123_789,
            op_index: 2,
            op_kind: ExperimentBusOpKind::WriteGated,
            status: 1,
            addr: 0xAF,
            write_value: 0,
            read_value: 0x3C,
        };
        let pkt = Packet::experiment_bus_op(0x4C, bus_op);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode bus op");
        assert_eq!(
            got.decode_experiment_record(),
            Some(ExperimentRecord::BusOp(bus_op))
        );

        let event = ExperimentEventRecord {
            trial_id: 7,
            timestamp_us: 124_000,
            event: ExperimentEventKind::CommandComplete,
            status: 0,
            flags: 0x03,
        };
        let pkt = Packet::experiment_event(0x4D, event);
        let raw = pkt.encode();
        let got = Packet::decode(&raw[..pkt.encoded_len()]).expect("decode event");
        assert_eq!(
            got.decode_experiment_record(),
            Some(ExperimentRecord::Event(event))
        );
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
        assert_eq!(packed, [0x34, 0xF1, 0x03]);
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
