use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gpiod::{Chip, Edge, EdgeDetect, Options};

use crate::homing::{Axis, Sensor, SensorStates};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HallLineConfig {
    pub line: u32,
    pub active_low: bool,
}

impl HallLineConfig {
    pub fn new(line: u32, active_low: bool) -> Self {
        Self { line, active_low }
    }

    fn logical_from_raw(self, raw_high: bool) -> bool {
        if self.active_low {
            !raw_high
        } else {
            raw_high
        }
    }

    fn logical_from_edge(self, edge: Edge) -> bool {
        match edge {
            Edge::Rising => !self.active_low,
            Edge::Falling => self.active_low,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HallGpioConfig {
    pub chip: String,
    pub x_coarse: HallLineConfig,
    pub x_home: HallLineConfig,
    pub z_coarse: HallLineConfig,
    pub z_home: HallLineConfig,
}

impl Default for HallGpioConfig {
    fn default() -> Self {
        Self {
            chip: "gpiochip0".to_string(),
            x_coarse: HallLineConfig::new(17, true),
            x_home: HallLineConfig::new(27, true),
            z_coarse: HallLineConfig::new(22, true),
            z_home: HallLineConfig::new(23, true),
        }
    }
}

impl HallGpioConfig {
    pub fn lines(&self) -> [u32; HALL_INPUT_COUNT] {
        [
            self.x_coarse.line,
            self.x_home.line,
            self.z_coarse.line,
            self.z_home.line,
        ]
    }

    pub fn line_config(&self, input: HallInput) -> HallLineConfig {
        match input {
            HallInput::XCoarse => self.x_coarse,
            HallInput::XHome => self.x_home,
            HallInput::ZCoarse => self.z_coarse,
            HallInput::ZHome => self.z_home,
        }
    }

    fn input_for_index(index: usize) -> Option<HallInput> {
        match index {
            0 => Some(HallInput::XCoarse),
            1 => Some(HallInput::XHome),
            2 => Some(HallInput::ZCoarse),
            3 => Some(HallInput::ZHome),
            _ => None,
        }
    }

    fn input_for_line(&self, line: u8) -> Option<HallInput> {
        let line = u32::from(line);
        if self.x_coarse.line == line {
            Some(HallInput::XCoarse)
        } else if self.x_home.line == line {
            Some(HallInput::XHome)
        } else if self.z_coarse.line == line {
            Some(HallInput::ZCoarse)
        } else if self.z_home.line == line {
            Some(HallInput::ZHome)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HallInput {
    XCoarse,
    XHome,
    ZCoarse,
    ZHome,
}

impl HallInput {
    pub fn axis(self) -> Axis {
        match self {
            Self::XCoarse | Self::XHome => Axis::X,
            Self::ZCoarse | Self::ZHome => Axis::Z,
        }
    }

    pub fn sensor(self) -> Sensor {
        match self {
            Self::XCoarse | Self::ZCoarse => Sensor::Coarse,
            Self::XHome | Self::ZHome => Sensor::Home,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::XCoarse => "x_coarse",
            Self::XHome => "x_home",
            Self::ZCoarse => "z_coarse",
            Self::ZHome => "z_home",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HallStates {
    pub x: SensorStates,
    pub z: SensorStates,
}

impl HallStates {
    pub fn get_input(self, input: HallInput) -> bool {
        match input {
            HallInput::XCoarse => self.x.coarse,
            HallInput::XHome => self.x.home,
            HallInput::ZCoarse => self.z.coarse,
            HallInput::ZHome => self.z.home,
        }
    }

    fn set_input(&mut self, input: HallInput, active: bool) {
        match input {
            HallInput::XCoarse => self.x.coarse = active,
            HallInput::XHome => self.x.home = active,
            HallInput::ZCoarse => self.z.coarse = active,
            HallInput::ZHome => self.z.home = active,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HallEvent {
    pub input: HallInput,
    pub axis: Axis,
    pub sensor: Sensor,
    pub active: bool,
    pub timestamp: Duration,
}

#[derive(Debug)]
struct SharedHallState {
    states: HallStates,
    events: VecDeque<HallEvent>,
    read_error: Option<String>,
}

impl SharedHallState {
    fn new(states: HallStates) -> Self {
        Self {
            states,
            events: VecDeque::new(),
            read_error: None,
        }
    }

    fn apply_event(&mut self, event: HallEvent) {
        self.states.set_input(event.input, event.active);
        self.events.push_back(event);
    }
}

pub struct HallGpioMonitor {
    shared: Arc<Mutex<SharedHallState>>,
    _worker: thread::JoinHandle<()>,
}

impl HallGpioMonitor {
    pub fn open(config: HallGpioConfig) -> io::Result<Self> {
        let chip = Chip::new(config.chip.as_str())?;
        let opts = Options::input(config.lines())
            .edge(EdgeDetect::Both)
            .consumer("fredctl-hall");
        let mut lines = chip.request_lines(opts)?;
        let raw_values = lines.get_values([false; HALL_INPUT_COUNT])?;
        let initial = states_from_raw_values(&config, raw_values);
        let shared = Arc::new(Mutex::new(SharedHallState::new(initial)));
        let worker_shared = Arc::clone(&shared);

        let worker = thread::spawn(move || loop {
            match lines.read_event() {
                Ok(event) => {
                    let Some(input) = config.input_for_line(event.line) else {
                        continue;
                    };
                    let line_config = config.line_config(input);
                    let hall_event = HallEvent {
                        input,
                        axis: input.axis(),
                        sensor: input.sensor(),
                        active: line_config.logical_from_edge(event.edge),
                        timestamp: event.time,
                    };
                    if let Ok(mut state) = worker_shared.lock() {
                        state.apply_event(hall_event);
                    }
                }
                Err(err) => {
                    if let Ok(mut state) = worker_shared.lock() {
                        state.read_error = Some(err.to_string());
                    }
                    break;
                }
            }
        });

        Ok(Self {
            shared,
            _worker: worker,
        })
    }

    pub fn states(&self) -> io::Result<HallStates> {
        let state = self.lock_state()?;
        if let Some(err) = &state.read_error {
            return Err(io::Error::other(format!("GPIO event read failed: {err}")));
        }
        Ok(state.states)
    }

    pub fn drain_events(&self) -> io::Result<Vec<HallEvent>> {
        let mut state = self.lock_state()?;
        if let Some(err) = &state.read_error {
            return Err(io::Error::other(format!("GPIO event read failed: {err}")));
        }
        Ok(state.events.drain(..).collect())
    }

    fn lock_state(&self) -> io::Result<std::sync::MutexGuard<'_, SharedHallState>> {
        self.shared
            .lock()
            .map_err(|_| io::Error::other("GPIO monitor state lock poisoned"))
    }
}

const HALL_INPUT_COUNT: usize = 4;

fn states_from_raw_values(
    config: &HallGpioConfig,
    raw_values: [bool; HALL_INPUT_COUNT],
) -> HallStates {
    let mut states = HallStates::default();
    for (index, raw_high) in raw_values.into_iter().enumerate() {
        let Some(input) = HallGpioConfig::input_for_index(index) else {
            continue;
        };
        let line_config = config.line_config(input);
        states.set_input(input, line_config.logical_from_raw(raw_high));
    }
    states
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gpiod::Edge;

    use super::{
        states_from_raw_values, HallGpioConfig, HallInput, HallLineConfig, HallStates,
        SharedHallState,
    };
    use crate::homing::{Axis, Sensor, SensorStates};

    #[test]
    fn active_low_mapping_treats_raw_low_as_active() {
        let config = HallGpioConfig {
            chip: "gpiochip0".to_string(),
            x_coarse: HallLineConfig::new(1, true),
            x_home: HallLineConfig::new(2, true),
            z_coarse: HallLineConfig::new(3, true),
            z_home: HallLineConfig::new(4, true),
        };

        let states = states_from_raw_values(&config, [false, true, false, true]);
        assert_eq!(
            states,
            HallStates {
                x: SensorStates {
                    coarse: true,
                    home: false,
                },
                z: SensorStates {
                    coarse: true,
                    home: false,
                },
            }
        );
    }

    #[test]
    fn edge_polarity_tracks_active_low_and_active_high() {
        assert!(HallLineConfig::new(1, true).logical_from_edge(Edge::Falling));
        assert!(!HallLineConfig::new(1, true).logical_from_edge(Edge::Rising));
        assert!(HallLineConfig::new(1, false).logical_from_edge(Edge::Rising));
        assert!(!HallLineConfig::new(1, false).logical_from_edge(Edge::Falling));
    }

    #[test]
    fn shared_state_updates_axis_sensor_state_from_event() {
        let mut shared = SharedHallState::new(HallStates::default());
        shared.apply_event(super::HallEvent {
            input: HallInput::ZHome,
            axis: Axis::Z,
            sensor: Sensor::Home,
            active: true,
            timestamp: Duration::from_millis(42),
        });

        assert!(shared.states.z.home);
        assert_eq!(shared.events.len(), 1);
        assert_eq!(shared.events[0].input, HallInput::ZHome);
    }
}
