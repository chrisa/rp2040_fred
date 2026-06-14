#![expect(
    dead_code,
    reason = "test includes firmware decoder without exercising every entrypoint"
)]

mod resources {
    pub const FRED_PIN: u32 = 16;
    pub const ONE_MHZ_PIN: u32 = 17;
    pub const READ_WRITE_PIN: u32 = 18;
}

#[path = "../src/decoder.rs"]
mod decoder;

use decoder::{FeedbackCommand, FeedbackDecoder, MissingBcdSnapshotPolicy};

const FEEDBACK_COMMANDS: [u8; 10] = [0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0D, 0x0C];

#[derive(Clone, Copy)]
struct FeedbackValues {
    x_sign: u8,
    x_pairs: [u8; 3],
    z_sign: u8,
    z_pairs: [u8; 3],
    rpm_pairs: [u8; 2],
}

impl FeedbackValues {
    const fn new(
        x_sign: u8,
        x_pairs: [u8; 3],
        z_sign: u8,
        z_pairs: [u8; 3],
        rpm_pairs: [u8; 2],
    ) -> Self {
        Self {
            x_sign,
            x_pairs,
            z_sign,
            z_pairs,
            rpm_pairs,
        }
    }

    fn response_for(self, cmd: u8) -> u8 {
        match cmd {
            0x03 => self.x_sign,
            0x02 => self.x_pairs[0],
            0x01 => self.x_pairs[1],
            0x00 => self.x_pairs[2],
            0x07 => self.z_sign,
            0x06 => self.z_pairs[0],
            0x05 => self.z_pairs[1],
            0x04 => self.z_pairs[2],
            0x0D => self.rpm_pairs[0],
            0x0C => self.rpm_pairs[1],
            _ => 0,
        }
    }
}

fn feed_snapshot(
    decoder: &mut FeedbackDecoder,
    start_index: u64,
    values: FeedbackValues,
    policy: MissingBcdSnapshotPolicy,
) -> Result<decoder::FeedbackSnapshot, &'static str> {
    let mut result = Err("no feedback commands");

    for (offset, cmd) in FEEDBACK_COMMANDS.iter().copied().enumerate() {
        result = decoder.ingest_command_with_policy(
            FeedbackCommand::from_master(
                start_index + offset as u64,
                cmd,
                values.response_for(cmd),
                false,
            ),
            policy,
        );
    }

    result
}

fn prime_previous(decoder: &mut FeedbackDecoder) {
    let values = FeedbackValues::new(0, [0x00, 0x12, 0x34], 1, [0x00, 0x56, 0x78], [0x12, 0x30]);

    for i in 0..4 {
        let snapshot = feed_snapshot(
            decoder,
            i * FEEDBACK_COMMANDS.len() as u64,
            values,
            MissingBcdSnapshotPolicy::Current,
        )
        .expect("valid feedback snapshot");

        assert_eq!(snapshot.x.count(), 1234);
        assert_eq!(snapshot.z.count(), -5678);
    }

    let snapshot = feed_snapshot(
        decoder,
        4 * FEEDBACK_COMMANDS.len() as u64,
        values,
        MissingBcdSnapshotPolicy::Current,
    )
    .expect("valid feedback snapshot");
    assert_eq!(snapshot.s.rpm(), 1230);
}

#[test]
fn valid_zero_bcd_is_a_real_zero_snapshot() {
    let mut decoder = FeedbackDecoder::new();
    let values = FeedbackValues::new(0, [0x00, 0x00, 0x00], 0, [0x00, 0x00, 0x00], [0x00, 0x00]);

    let snapshot = feed_snapshot(&mut decoder, 0, values, MissingBcdSnapshotPolicy::Current)
        .expect("zero feedback snapshot");

    assert_eq!(snapshot.x.count(), 0);
    assert_eq!(snapshot.z.count(), 0);
    assert_eq!(snapshot.s.rpm(), 0);
}

#[test]
fn current_policy_keeps_existing_axis_drop_behavior() {
    let mut decoder = FeedbackDecoder::new();
    prime_previous(&mut decoder);

    let values = FeedbackValues::new(0, [0x00, 0xA0, 0x00], 1, [0x00, 0x56, 0x78], [0x12, 0x30]);
    let result = feed_snapshot(&mut decoder, 50, values, MissingBcdSnapshotPolicy::Current);

    assert!(result.is_err());
}

#[test]
fn hold_previous_policy_fills_missing_axis_values() {
    let mut decoder = FeedbackDecoder::new();
    prime_previous(&mut decoder);

    let values = FeedbackValues::new(0, [0x00, 0xA0, 0x00], 1, [0x00, 0x56, 0x78], [0x12, 0x30]);
    let snapshot = feed_snapshot(
        &mut decoder,
        50,
        values,
        MissingBcdSnapshotPolicy::HoldPrevious,
    )
    .expect("held feedback snapshot");

    assert_eq!(snapshot.x.count(), 1234);
    assert_eq!(snapshot.z.count(), -5678);
    assert_eq!(snapshot.s.rpm(), 1230);
}

#[test]
fn hold_previous_policy_requires_a_prior_axis_value() {
    let mut decoder = FeedbackDecoder::new();

    let values = FeedbackValues::new(0, [0x00, 0xA0, 0x00], 0, [0x00, 0x00, 0x00], [0x00, 0x00]);
    let result = feed_snapshot(
        &mut decoder,
        0,
        values,
        MissingBcdSnapshotPolicy::HoldPrevious,
    );

    assert!(result.is_err());
}

#[test]
fn drop_incomplete_policy_rejects_missing_axis_values() {
    let mut decoder = FeedbackDecoder::new();
    prime_previous(&mut decoder);

    let values = FeedbackValues::new(0, [0x00, 0xA0, 0x00], 1, [0x00, 0x56, 0x78], [0x12, 0x30]);
    let result = feed_snapshot(
        &mut decoder,
        50,
        values,
        MissingBcdSnapshotPolicy::DropIncomplete,
    );

    assert!(result.is_err());
}

#[test]
fn hold_previous_policy_fills_missing_spindle_values() {
    let mut decoder = FeedbackDecoder::new();
    prime_previous(&mut decoder);

    let values = FeedbackValues::new(0, [0x00, 0x12, 0x34], 1, [0x00, 0x56, 0x78], [0x12, 0xA0]);
    let snapshot = feed_snapshot(
        &mut decoder,
        50,
        values,
        MissingBcdSnapshotPolicy::HoldPrevious,
    )
    .expect("held feedback snapshot");

    assert_eq!(snapshot.x.count(), 1234);
    assert_eq!(snapshot.z.count(), -5678);
    assert_eq!(snapshot.s.rpm(), 1230);
}

#[test]
fn drop_incomplete_policy_rejects_missing_spindle_values() {
    let mut decoder = FeedbackDecoder::new();
    prime_previous(&mut decoder);

    let values = FeedbackValues::new(0, [0x00, 0x12, 0x34], 1, [0x00, 0x56, 0x78], [0x12, 0xA0]);
    let result = feed_snapshot(
        &mut decoder,
        50,
        values,
        MissingBcdSnapshotPolicy::DropIncomplete,
    );

    assert!(result.is_err());
}
