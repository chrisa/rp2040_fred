use std::fmt;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Z,
}

impl Axis {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "x" | "X" => Ok(Self::X),
            "z" | "Z" => Ok(Self::Z),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown homing axis: {value}"),
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Z => "z",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensor {
    Coarse,
    Home,
}

impl Sensor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Coarse => "coarse",
            Self::Home => "home",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SensorStates {
    pub coarse: bool,
    pub home: bool,
}

impl SensorStates {
    pub fn get(self, sensor: Sensor) -> bool {
        match sensor {
            Sensor::Coarse => self.coarse,
            Sensor::Home => self.home,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisObservation {
    pub axis: Axis,
    pub time_s: f64,
    pub position_mm: f64,
    pub sensors: SensorStates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    Rapid,
    Feed,
}

impl MoveKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rapid => "rapid",
            Self::Feed => "feed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveCommand {
    pub axis: Axis,
    pub delta_mm: f64,
    pub kind: MoveKind,
    pub rate_mm_min: f64,
}

impl MoveCommand {
    pub fn positive(axis: Axis, distance_mm: f64, kind: MoveKind, rate_mm_min: f64) -> Self {
        Self {
            axis,
            delta_mm: distance_mm.abs(),
            kind,
            rate_mm_min,
        }
    }

    pub fn negative(axis: Axis, distance_mm: f64, kind: MoveKind, rate_mm_min: f64) -> Self {
        Self {
            axis,
            delta_mm: -distance_mm.abs(),
            kind,
            rate_mm_min,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TelemetrySample {
    pub time_s: f64,
    pub axis: Axis,
    pub position_mm: Option<f64>,
    pub dropped: bool,
    pub stale: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorEdge {
    pub time_s: f64,
    pub axis: Axis,
    pub sensor: Sensor,
    pub active: bool,
    pub position_mm: f64,
    pub true_position_mm: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoveObservation {
    pub command: MoveCommand,
    pub start_time_s: f64,
    pub end_time_s: f64,
    pub start_position_mm: f64,
    pub end_position_mm: f64,
    pub start_sensors: SensorStates,
    pub end_sensors: SensorStates,
    pub telemetry: Vec<TelemetrySample>,
    pub edges: Vec<SensorEdge>,
    pub hard_stop_contact: bool,
}

impl MoveObservation {
    pub fn first_edge(&self, sensor: Sensor, active: bool) -> Option<SensorEdge> {
        self.edges
            .iter()
            .copied()
            .find(|edge| edge.sensor == sensor && edge.active == active)
    }
}

pub trait HomingBackend {
    fn observe_axis(&mut self, axis: Axis) -> io::Result<AxisObservation>;
    fn execute_move(&mut self, command: MoveCommand) -> io::Result<MoveObservation>;
    fn set_axis_offset(&mut self, axis: Axis, offset_mm: f64) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HomingConfig {
    pub hard_min_mm: f64,
    pub hard_max_mm: f64,
    pub coarse_edge_mm: f64,
    pub home_edge_mm: f64,
    pub software_min_mm: f64,
    pub software_max_mm: f64,
    pub park_position_mm: f64,
    pub fast_block_mm: f64,
    pub fast_rate_mm_min: f64,
    pub near_block_mm: f64,
    pub near_rate_mm_min: f64,
    pub release_block_mm: f64,
    pub release_clearance_mm: f64,
    pub release_rate_mm_min: f64,
    pub max_release_mm: f64,
    pub max_search_mm: f64,
    pub max_near_mm: f64,
    pub latch_backoff_mm: f64,
    pub latch_block_mm: f64,
    pub latch_rate_mm_min: f64,
    pub latch_repeats: usize,
    pub max_latch_search_mm: f64,
}

impl Default for HomingConfig {
    fn default() -> Self {
        Self {
            hard_min_mm: 0.0,
            hard_max_mm: 100.0,
            coarse_edge_mm: 85.0,
            home_edge_mm: 98.0,
            software_min_mm: 1.0,
            software_max_mm: 99.0,
            park_position_mm: 96.0,
            fast_block_mm: 10.0,
            fast_rate_mm_min: 500.0,
            near_block_mm: 1.0,
            near_rate_mm_min: 250.0,
            release_block_mm: 1.0,
            release_clearance_mm: 1.0,
            release_rate_mm_min: 250.0,
            max_release_mm: 8.0,
            max_search_mm: 110.0,
            max_near_mm: 18.0,
            latch_backoff_mm: 1.0,
            latch_block_mm: 0.25,
            latch_rate_mm_min: 25.0,
            latch_repeats: 3,
            max_latch_search_mm: 4.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomingResult {
    pub axis: Axis,
    pub start_time_s: f64,
    pub end_time_s: f64,
    pub start_position_mm: f64,
    pub final_position_mm: f64,
    pub home_datum_mm: f64,
    pub coordinate_offset_mm: f64,
    pub latch_edges_mm: Vec<f64>,
    pub moves: Vec<MoveObservation>,
}

impl HomingResult {
    pub fn total_time_s(&self) -> f64 {
        self.end_time_s - self.start_time_s
    }

    pub fn move_count(&self) -> usize {
        self.moves.len()
    }

    pub fn hard_stop_contact(&self) -> bool {
        self.moves.iter().any(|movement| movement.hard_stop_contact)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HomingError {
    Backend(String),
    InvalidConfig(&'static str),
    HardStopContact {
        phase: &'static str,
        observation: MoveObservation,
    },
    HomeReleaseFailed,
    HomeStillActiveAfterBackoff,
    SearchExceeded,
    HomeSeenBeforeCoarse,
    HomeNotSeen,
    LatchFailed,
}

impl fmt::Display for HomingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(message) => write!(f, "backend error: {message}"),
            Self::InvalidConfig(message) => write!(f, "invalid homing config: {message}"),
            Self::HardStopContact { phase, .. } => {
                write!(f, "hard-stop contact during {phase}")
            }
            Self::HomeReleaseFailed => write!(f, "home sensor did not release during backoff"),
            Self::HomeStillActiveAfterBackoff => {
                write!(f, "home sensor is still active after latch backoff")
            }
            Self::SearchExceeded => write!(f, "homing search distance exceeded"),
            Self::HomeSeenBeforeCoarse => write!(f, "home sensor was seen before coarse sensor"),
            Self::HomeNotSeen => write!(f, "home sensor was not seen during near-home search"),
            Self::LatchFailed => write!(f, "slow latch failed to find the home edge"),
        }
    }
}

impl std::error::Error for HomingError {}

pub fn home_axis<B: HomingBackend>(
    backend: &mut B,
    axis: Axis,
    config: HomingConfig,
) -> Result<HomingResult, HomingError> {
    validate_config(config)?;

    let initial = observe(backend, axis)?;
    let mut moves = Vec::new();
    let mut state = initial;

    if state.sensors.home {
        state = release_home(backend, axis, config, &mut moves)?;
    }

    if !state.sensors.coarse {
        state = fast_search(backend, axis, config, &mut moves)?;
    }

    if !state.sensors.home {
        let _ = near_home_search(backend, axis, config, &mut moves)?;
    }

    let state = latch_backoff(backend, axis, config, &mut moves)?;
    if state.sensors.home {
        return Err(HomingError::HomeStillActiveAfterBackoff);
    }

    let latch_edges_mm = slow_latch(backend, axis, config, &mut moves)?;
    let home_datum_mm = median(&latch_edges_mm).ok_or(HomingError::LatchFailed)?;
    let coordinate_offset_mm = config.home_edge_mm - home_datum_mm;
    backend
        .set_axis_offset(axis, coordinate_offset_mm)
        .map_err(|err| HomingError::Backend(err.to_string()))?;

    let parked = park_axis(backend, axis, config, coordinate_offset_mm, &mut moves)?;

    Ok(HomingResult {
        axis,
        start_time_s: initial.time_s,
        end_time_s: parked.time_s,
        start_position_mm: initial.position_mm,
        final_position_mm: parked.position_mm,
        home_datum_mm,
        coordinate_offset_mm,
        latch_edges_mm,
        moves,
    })
}

fn validate_config(config: HomingConfig) -> Result<(), HomingError> {
    if config.hard_min_mm >= config.hard_max_mm {
        return Err(HomingError::InvalidConfig(
            "hard minimum must be lower than hard maximum",
        ));
    }
    if !(config.hard_min_mm < config.coarse_edge_mm && config.coarse_edge_mm < config.home_edge_mm)
    {
        return Err(HomingError::InvalidConfig(
            "coarse edge must be between hard minimum and home edge",
        ));
    }
    if !(config.home_edge_mm < config.hard_max_mm) {
        return Err(HomingError::InvalidConfig(
            "home edge must be below hard maximum",
        ));
    }
    if config.latch_repeats == 0 {
        return Err(HomingError::InvalidConfig(
            "at least one latch repeat is required",
        ));
    }
    Ok(())
}

fn observe<B: HomingBackend>(backend: &mut B, axis: Axis) -> Result<AxisObservation, HomingError> {
    backend
        .observe_axis(axis)
        .map_err(|err| HomingError::Backend(err.to_string()))
}

fn release_home<B: HomingBackend>(
    backend: &mut B,
    axis: Axis,
    config: HomingConfig,
    moves: &mut Vec<MoveObservation>,
) -> Result<AxisObservation, HomingError> {
    let mut released_mm = 0.0;
    let mut state = observe(backend, axis)?;

    while state.sensors.home {
        if released_mm >= config.max_release_mm {
            return Err(HomingError::HomeReleaseFailed);
        }
        let movement = execute_checked(
            backend,
            MoveCommand::negative(
                axis,
                config.release_block_mm,
                MoveKind::Feed,
                config.release_rate_mm_min,
            ),
            "home release",
        )?;
        released_mm += config.release_block_mm.abs();
        state = observation_from_move(axis, &movement);
        moves.push(movement);
    }

    let movement = execute_checked(
        backend,
        MoveCommand::negative(
            axis,
            config.release_clearance_mm,
            MoveKind::Feed,
            config.release_rate_mm_min,
        ),
        "home release clearance",
    )?;
    state = observation_from_move(axis, &movement);
    moves.push(movement);
    Ok(state)
}

fn fast_search<B: HomingBackend>(
    backend: &mut B,
    axis: Axis,
    config: HomingConfig,
    moves: &mut Vec<MoveObservation>,
) -> Result<AxisObservation, HomingError> {
    let mut searched_mm = 0.0;
    let mut state = observe(backend, axis)?;

    while !state.sensors.coarse {
        if searched_mm >= config.max_search_mm {
            return Err(HomingError::SearchExceeded);
        }

        let movement = execute_checked(
            backend,
            MoveCommand::positive(
                axis,
                config.fast_block_mm,
                MoveKind::Rapid,
                config.fast_rate_mm_min,
            ),
            "fast search",
        )?;

        if movement.first_edge(Sensor::Home, true).is_some() || movement.end_sensors.home {
            return Err(HomingError::HomeSeenBeforeCoarse);
        }

        searched_mm += config.fast_block_mm.abs();
        state = observation_from_move(axis, &movement);
        moves.push(movement);
    }

    Ok(state)
}

fn near_home_search<B: HomingBackend>(
    backend: &mut B,
    axis: Axis,
    config: HomingConfig,
    moves: &mut Vec<MoveObservation>,
) -> Result<AxisObservation, HomingError> {
    let mut searched_mm = 0.0;
    let mut state = observe(backend, axis)?;

    while !state.sensors.home {
        if searched_mm >= config.max_near_mm {
            return Err(HomingError::HomeNotSeen);
        }

        let movement = execute_checked(
            backend,
            MoveCommand::positive(
                axis,
                config.near_block_mm,
                MoveKind::Feed,
                config.near_rate_mm_min,
            ),
            "near-home search",
        )?;
        searched_mm += config.near_block_mm.abs();
        state = observation_from_move(axis, &movement);
        moves.push(movement);
    }

    Ok(state)
}

fn latch_backoff<B: HomingBackend>(
    backend: &mut B,
    axis: Axis,
    config: HomingConfig,
    moves: &mut Vec<MoveObservation>,
) -> Result<AxisObservation, HomingError> {
    let movement = execute_checked(
        backend,
        MoveCommand::negative(
            axis,
            config.latch_backoff_mm,
            MoveKind::Feed,
            config.near_rate_mm_min,
        ),
        "latch backoff",
    )?;
    let state = observation_from_move(axis, &movement);
    moves.push(movement);
    Ok(state)
}

fn slow_latch<B: HomingBackend>(
    backend: &mut B,
    axis: Axis,
    config: HomingConfig,
    moves: &mut Vec<MoveObservation>,
) -> Result<Vec<f64>, HomingError> {
    let mut latch_edges_mm = Vec::new();

    for attempt in 0..config.latch_repeats {
        let mut searched_mm = 0.0;
        let mut found_edge = None;

        while found_edge.is_none() {
            if searched_mm >= config.max_latch_search_mm {
                return Err(HomingError::LatchFailed);
            }

            let movement = execute_checked(
                backend,
                MoveCommand::positive(
                    axis,
                    config.latch_block_mm,
                    MoveKind::Feed,
                    config.latch_rate_mm_min,
                ),
                "slow latch",
            )?;

            if let Some(edge) = movement.first_edge(Sensor::Home, true) {
                found_edge = Some(edge.position_mm);
            } else if movement.end_sensors.home {
                return Err(HomingError::LatchFailed);
            }

            searched_mm += config.latch_block_mm.abs();
            moves.push(movement);
        }

        if let Some(edge_mm) = found_edge {
            latch_edges_mm.push(edge_mm);
        }

        if attempt + 1 < config.latch_repeats {
            let state = latch_backoff(backend, axis, config, moves)?;
            if state.sensors.home {
                return Err(HomingError::HomeStillActiveAfterBackoff);
            }
        }
    }

    Ok(latch_edges_mm)
}

fn park_axis<B: HomingBackend>(
    backend: &mut B,
    axis: Axis,
    config: HomingConfig,
    _coordinate_offset_mm: f64,
    moves: &mut Vec<MoveObservation>,
) -> Result<AxisObservation, HomingError> {
    let state = observe(backend, axis)?;
    let current_machine_position = state.position_mm;
    let delta = config.park_position_mm - current_machine_position;

    if delta.abs() > f64::EPSILON {
        let movement = execute_checked(
            backend,
            MoveCommand {
                axis,
                delta_mm: delta,
                kind: MoveKind::Feed,
                rate_mm_min: config.near_rate_mm_min,
            },
            "post-home park",
        )?;
        let state = AxisObservation {
            axis,
            time_s: movement.end_time_s,
            position_mm: movement.end_position_mm,
            sensors: movement.end_sensors,
        };
        moves.push(movement);
        Ok(state)
    } else {
        Ok(state)
    }
}

fn execute_checked<B: HomingBackend>(
    backend: &mut B,
    command: MoveCommand,
    phase: &'static str,
) -> Result<MoveObservation, HomingError> {
    let movement = backend
        .execute_move(command)
        .map_err(|err| HomingError::Backend(err.to_string()))?;
    if movement.hard_stop_contact {
        return Err(HomingError::HardStopContact {
            phase,
            observation: movement,
        });
    }
    Ok(movement)
}

fn observation_from_move(axis: Axis, movement: &MoveObservation) -> AxisObservation {
    AxisObservation {
        axis,
        time_s: movement.end_time_s,
        position_mm: movement.end_position_mm,
        sensors: movement.end_sensors,
    }
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        Some((sorted[mid - 1] + sorted[mid]) / 2.0)
    } else {
        sorted.get(mid).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::median;

    #[test]
    fn median_handles_odd_and_even_counts() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }
}
