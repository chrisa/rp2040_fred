use crate::protocol::{DroProtocolEngine, FredReply};

pub const DRO_CADENCE: [u8; 10] = [0x03, 0x02, 0x01, 0x00, 0x07, 0x06, 0x05, 0x04, 0x0D, 0x0C];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MockBusFrame {
    pub cmd_fc80: u8,
    pub status_fcf0: u8,
    pub response_fcf1: u8,
}

pub struct MockBusRunner {
    engine: DroProtocolEngine,
    idx: usize,
}

impl MockBusRunner {
    pub const fn new() -> Self {
        Self {
            engine: DroProtocolEngine::new(),
            idx: 0,
        }
    }

    pub fn step(&mut self) -> MockBusFrame {
        let cmd = DRO_CADENCE[self.idx];
        self.idx = (self.idx + 1) % DRO_CADENCE.len();

        self.engine.step_telemetry();
        let FredReply {
            status_fcf0,
            response_fcf1,
        } = self.engine.on_command(cmd);

        MockBusFrame {
            cmd_fc80: cmd,
            status_fcf0,
            response_fcf1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MockBusRunner, DRO_CADENCE};

    #[test]
    fn cadence_repeats_in_expected_order() {
        let mut sim = MockBusRunner::new();
        for i in 0..(DRO_CADENCE.len() * 3) {
            let frame = sim.step();
            assert_eq!(frame.cmd_fc80, DRO_CADENCE[i % DRO_CADENCE.len()]);
        }
    }

    #[test]
    fn status_is_ready_in_mock_path() {
        let mut sim = MockBusRunner::new();
        for _ in 0..40 {
            let frame = sim.step();
            assert_eq!(frame.status_fcf0, 0x00);
        }
    }

    #[test]
    fn speed_pair_is_sensible() {
        let mut sim = MockBusRunner::new();
        for _ in 0..8 {
            let _ = sim.step();
        }
        let hi = sim.step();
        let lo = sim.step();

        assert_eq!(hi.cmd_fc80, 0x0D);
        assert_eq!(lo.cmd_fc80, 0x0C);

        let rpm = ((hi.response_fcf1 as u16) << 8) | lo.response_fcf1 as u16;
        assert!((800..=2200).contains(&rpm));
    }
}
