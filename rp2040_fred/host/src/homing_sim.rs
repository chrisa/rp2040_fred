use std::fmt::Write as _;
use std::fs;
use std::io;

use crate::homing::{
    home_axis, Axis, AxisObservation, HomingBackend, HomingConfig, HomingError, HomingResult,
    MoveCommand, MoveObservation, Sensor, SensorEdge, SensorStates, TelemetrySample,
};

const DEFAULT_STARTS: [f64; 9] = [0.0, 25.0, 50.0, 84.0, 86.0, 90.0, 97.0, 98.5, 99.5];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HallSensorSim {
    pub edge_mm: f64,
    pub missing: bool,
    pub inverted: bool,
    pub stuck: Option<bool>,
    pub edge_jitter_mm: f64,
}

impl HallSensorSim {
    pub fn active(self, true_position_mm: f64, hard_max_mm: f64) -> bool {
        if let Some(stuck) = self.stuck {
            return stuck;
        }

        let physical_active =
            !self.missing && true_position_mm >= self.edge_mm && true_position_mm <= hard_max_mm;

        if self.inverted {
            !physical_active
        } else {
            physical_active
        }
    }

    fn crossing_edge_mm(self, crossing_index: u64) -> f64 {
        if self.edge_jitter_mm == 0.0 {
            return self.edge_mm;
        }

        let pattern = match crossing_index % 5 {
            0 => -0.5,
            1 => 0.25,
            2 => 0.0,
            3 => 0.5,
            _ => -0.25,
        };
        self.edge_mm + self.edge_jitter_mm * pattern
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TelemetrySimConfig {
    pub period_s: f64,
    pub delay_s: f64,
    pub drop_every: Option<u64>,
    pub stale_every: Option<u64>,
    pub quantize_mm: Option<f64>,
    pub noise_mm: f64,
}

impl Default for TelemetrySimConfig {
    fn default() -> Self {
        Self {
            period_s: 0.010,
            delay_s: 0.0,
            drop_every: None,
            stale_every: None,
            quantize_mm: None,
            noise_mm: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimConfig {
    pub hard_min_mm: f64,
    pub hard_max_mm: f64,
    pub coarse: HallSensorSim,
    pub home: HallSensorSim,
    pub telemetry: TelemetrySimConfig,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            hard_min_mm: 0.0,
            hard_max_mm: 100.0,
            coarse: HallSensorSim {
                edge_mm: 85.0,
                missing: false,
                inverted: false,
                stuck: None,
                edge_jitter_mm: 0.0,
            },
            home: HallSensorSim {
                edge_mm: 98.0,
                missing: false,
                inverted: false,
                stuck: None,
                edge_jitter_mm: 0.0,
            },
            telemetry: TelemetrySimConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisState {
    true_position_mm: f64,
    offset_mm: f64,
}

impl AxisState {
    fn reported_position_mm(self) -> f64 {
        self.true_position_mm + self.offset_mm
    }
}

#[derive(Clone, Debug)]
pub struct SimHomingBackend {
    config: SimConfig,
    x: AxisState,
    z: AxisState,
    time_s: f64,
    telemetry_index: u64,
    edge_index: u64,
    last_telemetry_position: Option<f64>,
}

impl SimHomingBackend {
    pub fn new(start_axis: Axis, start_position_mm: f64, config: SimConfig) -> Self {
        let start = start_position_mm.clamp(config.hard_min_mm, config.hard_max_mm);
        let mut backend = Self {
            config,
            x: AxisState {
                true_position_mm: 50.0,
                offset_mm: 0.0,
            },
            z: AxisState {
                true_position_mm: 50.0,
                offset_mm: 0.0,
            },
            time_s: 0.0,
            telemetry_index: 0,
            edge_index: 0,
            last_telemetry_position: None,
        };
        backend.axis_state_mut(start_axis).true_position_mm = start;
        backend
    }

    pub fn time_s(&self) -> f64 {
        self.time_s
    }

    fn axis_state(&self, axis: Axis) -> AxisState {
        match axis {
            Axis::X => self.x,
            Axis::Z => self.z,
        }
    }

    fn axis_state_mut(&mut self, axis: Axis) -> &mut AxisState {
        match axis {
            Axis::X => &mut self.x,
            Axis::Z => &mut self.z,
        }
    }

    fn sensor_states_for_position(&self, true_position_mm: f64) -> SensorStates {
        SensorStates {
            coarse: self
                .config
                .coarse
                .active(true_position_mm, self.config.hard_max_mm),
            home: self
                .config
                .home
                .active(true_position_mm, self.config.hard_max_mm),
        }
    }

    fn position_at_time(
        &self,
        start_true_mm: f64,
        delta_mm: f64,
        duration_s: f64,
        elapsed_s: f64,
    ) -> f64 {
        if duration_s == 0.0 {
            return start_true_mm;
        }
        let unclamped = start_true_mm + delta_mm * (elapsed_s / duration_s);
        unclamped.clamp(self.config.hard_min_mm, self.config.hard_max_mm)
    }

    fn telemetry_sample(
        &mut self,
        axis: Axis,
        time_s: f64,
        true_position_mm: f64,
        offset_mm: f64,
    ) -> TelemetrySample {
        self.telemetry_index = self.telemetry_index.wrapping_add(1);

        if self
            .config
            .telemetry
            .drop_every
            .is_some_and(|every| every != 0 && self.telemetry_index % every == 0)
        {
            return TelemetrySample {
                time_s: time_s + self.config.telemetry.delay_s,
                axis,
                position_mm: None,
                dropped: true,
                stale: false,
            };
        }

        let stale = self
            .config
            .telemetry
            .stale_every
            .is_some_and(|every| every != 0 && self.telemetry_index % every == 0);

        let mut position_mm = if stale {
            self.last_telemetry_position
                .unwrap_or(true_position_mm + offset_mm)
        } else {
            true_position_mm + offset_mm
        };

        if !stale && self.config.telemetry.noise_mm != 0.0 {
            let sign = if self.telemetry_index % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            position_mm += self.config.telemetry.noise_mm * sign;
        }

        if let Some(quantum) = self.config.telemetry.quantize_mm {
            if quantum > 0.0 {
                position_mm = (position_mm / quantum).round() * quantum;
            }
        }

        if !stale {
            self.last_telemetry_position = Some(position_mm);
        }

        TelemetrySample {
            time_s: time_s + self.config.telemetry.delay_s,
            axis,
            position_mm: Some(position_mm),
            dropped: false,
            stale,
        }
    }

    fn movement_edges(
        &mut self,
        axis: Axis,
        start_true_mm: f64,
        end_true_mm: f64,
        start_time_s: f64,
        duration_s: f64,
        delta_mm: f64,
        offset_mm: f64,
        telemetry: &[TelemetrySample],
    ) -> Vec<SensorEdge> {
        let mut edges = Vec::new();
        self.push_sensor_edges(
            &mut edges,
            axis,
            Sensor::Coarse,
            self.config.coarse,
            start_true_mm,
            end_true_mm,
            start_time_s,
            duration_s,
            delta_mm,
            offset_mm,
            telemetry,
        );
        self.push_sensor_edges(
            &mut edges,
            axis,
            Sensor::Home,
            self.config.home,
            start_true_mm,
            end_true_mm,
            start_time_s,
            duration_s,
            delta_mm,
            offset_mm,
            telemetry,
        );
        edges.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
        edges
    }

    fn push_sensor_edges(
        &mut self,
        edges: &mut Vec<SensorEdge>,
        axis: Axis,
        sensor: Sensor,
        sensor_sim: HallSensorSim,
        start_true_mm: f64,
        end_true_mm: f64,
        start_time_s: f64,
        duration_s: f64,
        delta_mm: f64,
        offset_mm: f64,
        telemetry: &[TelemetrySample],
    ) {
        if sensor_sim.missing || sensor_sim.stuck.is_some() || delta_mm == 0.0 {
            return;
        }

        self.edge_index = self.edge_index.wrapping_add(1);
        let edge_true_mm = sensor_sim.crossing_edge_mm(self.edge_index);
        let crosses_positive = start_true_mm < edge_true_mm && end_true_mm >= edge_true_mm;
        let crosses_negative = start_true_mm >= edge_true_mm && end_true_mm < edge_true_mm;
        if !crosses_positive && !crosses_negative {
            return;
        }

        let active = if sensor_sim.inverted {
            crosses_negative
        } else {
            crosses_positive
        };
        let fraction = (edge_true_mm - start_true_mm) / delta_mm;
        let edge_time_s = start_time_s + duration_s * fraction.clamp(0.0, 1.0);
        let estimated_position_mm =
            estimate_edge_position(edge_time_s, telemetry).unwrap_or(edge_true_mm + offset_mm);

        edges.push(SensorEdge {
            time_s: edge_time_s,
            axis,
            sensor,
            active,
            position_mm: estimated_position_mm,
            true_position_mm: Some(edge_true_mm),
        });
    }
}

impl HomingBackend for SimHomingBackend {
    fn observe_axis(&mut self, axis: Axis) -> io::Result<AxisObservation> {
        let state = self.axis_state(axis);
        Ok(AxisObservation {
            axis,
            time_s: self.time_s,
            position_mm: state.reported_position_mm(),
            sensors: self.sensor_states_for_position(state.true_position_mm),
        })
    }

    fn execute_move(&mut self, command: MoveCommand) -> io::Result<MoveObservation> {
        if command.rate_mm_min <= 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "simulated move rate must be positive",
            ));
        }

        let start_state = self.axis_state(command.axis);
        let start_true_mm = start_state.true_position_mm;
        let start_reported_mm = start_state.reported_position_mm();
        let start_sensors = self.sensor_states_for_position(start_true_mm);
        let start_time_s = self.time_s;
        let requested_end_true_mm = start_true_mm + command.delta_mm;
        let end_true_mm =
            requested_end_true_mm.clamp(self.config.hard_min_mm, self.config.hard_max_mm);
        let hard_stop_contact = requested_end_true_mm < self.config.hard_min_mm
            || requested_end_true_mm > self.config.hard_max_mm;
        let duration_s = (command.delta_mm.abs() / command.rate_mm_min) * 60.0;
        let end_time_s = start_time_s + duration_s;

        let mut telemetry = Vec::new();
        let mut sample_time_s = start_time_s + self.config.telemetry.period_s;
        while sample_time_s <= end_time_s + 1e-9 {
            let elapsed_s = sample_time_s - start_time_s;
            let true_position_mm =
                self.position_at_time(start_true_mm, command.delta_mm, duration_s, elapsed_s);
            telemetry.push(self.telemetry_sample(
                command.axis,
                sample_time_s,
                true_position_mm,
                start_state.offset_mm,
            ));
            sample_time_s += self.config.telemetry.period_s;
        }

        let edges = self.movement_edges(
            command.axis,
            start_true_mm,
            end_true_mm,
            start_time_s,
            duration_s,
            command.delta_mm,
            start_state.offset_mm,
            &telemetry,
        );

        self.time_s = end_time_s;
        self.axis_state_mut(command.axis).true_position_mm = end_true_mm;
        let end_sensors = self.sensor_states_for_position(end_true_mm);

        Ok(MoveObservation {
            command,
            start_time_s,
            end_time_s,
            start_position_mm: start_reported_mm,
            end_position_mm: end_true_mm + start_state.offset_mm,
            start_sensors,
            end_sensors,
            telemetry,
            edges,
            hard_stop_contact,
        })
    }

    fn set_axis_offset(&mut self, axis: Axis, offset_mm: f64) -> io::Result<()> {
        self.axis_state_mut(axis).offset_mm = offset_mm;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SimRunReport {
    pub axis: Axis,
    pub start_position_mm: f64,
    pub result: Result<HomingResult, HomingError>,
}

impl SimRunReport {
    pub fn success(&self) -> bool {
        self.result.is_ok()
    }
}

pub fn default_starts() -> &'static [f64] {
    &DEFAULT_STARTS
}

pub fn run_simulation(
    axis: Axis,
    start_position_mm: f64,
    homing_config: HomingConfig,
    sim_config: SimConfig,
) -> SimRunReport {
    let mut backend = SimHomingBackend::new(axis, start_position_mm, sim_config);
    let result = home_axis(&mut backend, axis, homing_config);
    SimRunReport {
        axis,
        start_position_mm,
        result,
    }
}

pub fn write_csv_report(path: &str, reports: &[SimRunReport]) -> io::Result<()> {
    let mut out = String::new();
    out.push_str("scenario,axis,start_mm,row_type,time_s,position_mm,move_kind,move_delta_mm,coarse,home,dropped,stale,event\n");

    for (scenario_index, report) in reports.iter().enumerate() {
        if let Ok(result) = &report.result {
            for movement in &result.moves {
                write!(
                    out,
                    "{scenario_index},{},{:.3},move_start,{:.6},{:.6},{},{:.6},{},{},,,\n",
                    report.axis.label(),
                    report.start_position_mm,
                    movement.start_time_s,
                    movement.start_position_mm,
                    movement.command.kind.label(),
                    movement.command.delta_mm,
                    u8::from(movement.start_sensors.coarse),
                    u8::from(movement.start_sensors.home),
                )
                .map_err(fmt_error)?;
                write!(
                    out,
                    "{scenario_index},{},{:.3},move_end,{:.6},{:.6},{},{:.6},{},{},,,{}\n",
                    report.axis.label(),
                    report.start_position_mm,
                    movement.end_time_s,
                    movement.end_position_mm,
                    movement.command.kind.label(),
                    movement.command.delta_mm,
                    u8::from(movement.end_sensors.coarse),
                    u8::from(movement.end_sensors.home),
                    if movement.hard_stop_contact {
                        "hard_stop"
                    } else {
                        ""
                    },
                )
                .map_err(fmt_error)?;
                for sample in &movement.telemetry {
                    write!(
                        out,
                        "{scenario_index},{},{:.3},telemetry,{:.6},{},,,,{},{},{}\n",
                        report.axis.label(),
                        report.start_position_mm,
                        sample.time_s,
                        sample
                            .position_mm
                            .map(|pos| format!("{pos:.6}"))
                            .unwrap_or_default(),
                        u8::from(sample.dropped),
                        u8::from(sample.stale),
                        "",
                    )
                    .map_err(fmt_error)?;
                }
                for edge in &movement.edges {
                    write!(
                        out,
                        "{scenario_index},{},{:.3},edge,{:.6},{:.6},,,,,,,{}:{}:{}\n",
                        report.axis.label(),
                        report.start_position_mm,
                        edge.time_s,
                        edge.position_mm,
                        edge.sensor.label(),
                        if edge.active { "active" } else { "inactive" },
                        edge.true_position_mm
                            .map(|pos| format!("{pos:.6}"))
                            .unwrap_or_default(),
                    )
                    .map_err(fmt_error)?;
                }
            }
        }
    }

    fs::write(path, out)
}

pub fn write_html_report(path: &str, reports: &[SimRunReport]) -> io::Result<()> {
    fs::write(path, html_report(reports))
}

pub fn html_report(reports: &[SimRunReport]) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><title>TCL125 Homing Simulation</title>");
    out.push_str("<style>body{font-family:system-ui,sans-serif;margin:24px;color:#1f2933;background:#f8fafc}section{background:#fff;border:1px solid #d9e2ec;border-radius:6px;margin:0 0 18px;padding:14px}svg{width:100%;height:auto;border:1px solid #d9e2ec;background:#fff}.ok{color:#147d64}.fault{color:#b42318}.muted{color:#627d98;font-size:12px}.moves{max-height:320px;overflow:auto;border:1px solid #e4e7eb;border-radius:6px}.moves table{width:100%;font-size:12px}table{border-collapse:collapse}td,th{padding:4px 8px;border-bottom:1px solid #e4e7eb;text-align:left}th{background:#f0f4f8;position:sticky;top:0}</style></head><body>");
    out.push_str("<h1>TCL125 Homing Simulation</h1>");
    out.push_str("<table><thead><tr><th>Axis</th><th>Start</th><th>Result</th><th>Time</th><th>Moves</th><th>Datum</th></tr></thead><tbody>");
    for report in reports {
        match &report.result {
            Ok(result) => {
                write!(
                    out,
                    "<tr><td>{}</td><td>{:.3} mm</td><td class=\"ok\">ok</td><td>{:.3} s</td><td>{}</td><td>{:.6} mm</td></tr>",
                    report.axis.label(),
                    report.start_position_mm,
                    result.total_time_s(),
                    result.move_count(),
                    result.home_datum_mm,
                )
                .ok();
            }
            Err(err) => {
                write!(
                    out,
                    "<tr><td>{}</td><td>{:.3} mm</td><td class=\"fault\">fault</td><td></td><td></td><td>{}</td></tr>",
                    report.axis.label(),
                    report.start_position_mm,
                    escape_html(&err.to_string()),
                )
                .ok();
            }
        }
    }
    out.push_str("</tbody></table>");

    for (index, report) in reports.iter().enumerate() {
        write!(
            out,
            "<section><h2>{}: axis {}, start {:.3} mm</h2>",
            index + 1,
            report.axis.label(),
            report.start_position_mm,
        )
        .ok();

        match &report.result {
            Ok(result) => {
                write!(
                    out,
                    "<p class=\"ok\">Success. Total time {:.3} s, moves {}, median datum {:.6} mm, offset {:+.6} mm, final {:.3} mm.</p>",
                    result.total_time_s(),
                    result.move_count(),
                    result.home_datum_mm,
                    result.coordinate_offset_mm,
                    result.final_position_mm,
                )
                .ok();
                out.push_str(&scenario_svg(result));
                out.push_str(&move_table(result));
            }
            Err(err) => {
                write!(
                    out,
                    "<p class=\"fault\">Fault: {}</p>",
                    escape_html(&err.to_string())
                )
                .ok();
                if let HomingError::HardStopContact { observation, .. } = err {
                    out.push_str(&fault_svg(observation));
                    out.push_str(&single_move_table(observation));
                }
            }
        }
        out.push_str("</section>");
    }

    out.push_str("</body></html>");
    out
}

fn scenario_svg(result: &HomingResult) -> String {
    let width = 1000.0;
    let height = 260.0;
    let left = 46.0;
    let right = 20.0;
    let top = 20.0;
    let sensor_y0 = 184.0;
    let sensor_h = 18.0;
    let t_min = result.start_time_s;
    let t_max = result.end_time_s.max(t_min + 1.0);
    let time_span = t_max - t_min;
    let plot_w = width - left - right;
    let x_for_time = |time_s: f64| left + ((time_s - t_min) / time_span).clamp(0.0, 1.0) * plot_w;
    let y_for_pos = |pos_mm: f64| top + (100.0 - pos_mm.clamp(0.0, 100.0)) * 1.25;

    let mut out = String::new();
    write!(
        out,
        "<svg viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\" aria-label=\"homing trace\">"
    )
    .ok();
    out.push_str("<text x=\"6\" y=\"24\" font-size=\"12\">100</text><text x=\"12\" y=\"148\" font-size=\"12\">0</text>");
    out.push_str("<line x1=\"46\" y1=\"20\" x2=\"980\" y2=\"20\" stroke=\"#d9e2ec\"/><line x1=\"46\" y1=\"145\" x2=\"980\" y2=\"145\" stroke=\"#d9e2ec\"/>");
    out.push_str("<text x=\"6\" y=\"198\" font-size=\"12\">coarse</text><text x=\"12\" y=\"226\" font-size=\"12\">home</text>");

    for movement in &result.moves {
        let x0 = x_for_time(movement.start_time_s);
        let x1 = x_for_time(movement.end_time_s);
        let fill = if movement.command.kind.label() == "rapid" {
            "#e0f2fe"
        } else {
            "#ecfccb"
        };
        write!(
            out,
            "<rect x=\"{x0:.2}\" y=\"20\" width=\"{:.2}\" height=\"125\" fill=\"{}\" opacity=\"0.35\"/>",
            (x1 - x0).max(1.0),
            fill,
        )
        .ok();
    }

    out.push_str("<polyline fill=\"none\" stroke=\"#1d4ed8\" stroke-width=\"2\" points=\"");
    for movement in &result.moves {
        write!(
            out,
            "{:.2},{:.2} {:.2},{:.2} ",
            x_for_time(movement.start_time_s),
            y_for_pos(movement.start_position_mm),
            x_for_time(movement.end_time_s),
            y_for_pos(movement.end_position_mm),
        )
        .ok();
    }
    out.push_str("\"/>");

    for movement in &result.moves {
        for sample in &movement.telemetry {
            let x = x_for_time(sample.time_s);
            if let Some(pos) = sample.position_mm {
                let y = y_for_pos(pos);
                let color = if sample.stale { "#f97316" } else { "#334e68" };
                write!(
                    out,
                    "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"1.4\" fill=\"{color}\" opacity=\"0.65\"/>"
                )
                .ok();
            } else if sample.dropped {
                write!(
                    out,
                    "<line x1=\"{x:.2}\" y1=\"150\" x2=\"{x:.2}\" y2=\"242\" stroke=\"#b42318\" stroke-width=\"1\" opacity=\"0.35\"/>"
                )
                .ok();
            }
        }

        draw_sensor_trace(
            &mut out,
            movement,
            Sensor::Coarse,
            sensor_y0,
            sensor_h,
            &x_for_time,
        );
        draw_sensor_trace(
            &mut out,
            movement,
            Sensor::Home,
            sensor_y0 + 28.0,
            sensor_h,
            &x_for_time,
        );

        for edge in &movement.edges {
            let x = x_for_time(edge.time_s);
            let color = match edge.sensor {
                Sensor::Coarse => "#0284c7",
                Sensor::Home => "#b45309",
            };
            write!(
                out,
                "<line x1=\"{x:.2}\" y1=\"20\" x2=\"{x:.2}\" y2=\"244\" stroke=\"{color}\" stroke-width=\"1.5\"/><text x=\"{:.2}\" y=\"{}\" font-size=\"10\" fill=\"{color}\">{} {}</text>",
                x + 3.0,
                if edge.sensor == Sensor::Coarse { 176 } else { 242 },
                edge.sensor.label(),
                if edge.active { "on" } else { "off" },
            )
            .ok();
        }
    }

    if let Some(median) = result.latch_edges_mm.get(result.latch_edges_mm.len() / 2) {
        write!(
            out,
            "<text x=\"54\" y=\"160\" font-size=\"12\" fill=\"#92400e\">median latch {:.6} mm</text>",
            median
        )
        .ok();
    }

    out.push_str("<text x=\"54\" y=\"256\" font-size=\"11\" fill=\"#627d98\">blue: position, dots: telemetry, red verticals: dropped telemetry, shaded: command blocks, vertical labels: Hall edges</text>");
    out.push_str("</svg>");
    out
}

fn fault_svg(observation: &MoveObservation) -> String {
    let result = HomingResult {
        axis: observation.command.axis,
        start_time_s: observation.start_time_s,
        end_time_s: observation.end_time_s,
        start_position_mm: observation.start_position_mm,
        final_position_mm: observation.end_position_mm,
        home_datum_mm: 0.0,
        coordinate_offset_mm: 0.0,
        latch_edges_mm: Vec::new(),
        moves: vec![observation.clone()],
    };
    scenario_svg(&result)
}

fn move_table(result: &HomingResult) -> String {
    moves_table(
        &result.moves,
        Some(format!("{:+.6} mm", result.coordinate_offset_mm)),
    )
}

fn single_move_table(observation: &MoveObservation) -> String {
    moves_table(std::slice::from_ref(observation), None)
}

fn moves_table(moves: &[MoveObservation], coordinate_offset: Option<String>) -> String {
    let mut out = String::new();
    out.push_str("<h3>Commanded Moves</h3>");
    if let Some(offset) = coordinate_offset {
        write!(
            out,
            "<p class=\"muted\">Final coordinate offset applied after latch: {}</p>",
            escape_html(&offset)
        )
        .ok();
    }
    out.push_str("<div class=\"moves\"><table><thead><tr><th>#</th><th>Axis</th><th>Mode</th><th>Rate mm/min</th><th>Offset mm</th><th>Start mm</th><th>End mm</th><th>Duration s</th><th>Telemetry</th><th>Edges</th><th>Hard stop</th></tr></thead><tbody>");

    for (index, movement) in moves.iter().enumerate() {
        let duration_s = movement.end_time_s - movement.start_time_s;
        let edge_labels = movement
            .edges
            .iter()
            .map(|edge| {
                format!(
                    "{} {} @ {:.6}",
                    edge.sensor.label(),
                    if edge.active { "on" } else { "off" },
                    edge.position_mm
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            out,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.3}</td><td>{:+.6}</td><td>{:.6}</td><td>{:.6}</td><td>{:.3}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            index + 1,
            movement.command.axis.label(),
            movement.command.kind.label(),
            movement.command.rate_mm_min,
            movement.command.delta_mm,
            movement.start_position_mm,
            movement.end_position_mm,
            duration_s,
            movement.telemetry.len(),
            escape_html(&edge_labels),
            if movement.hard_stop_contact { "yes" } else { "" },
        )
        .ok();
    }

    out.push_str("</tbody></table></div>");
    out
}

fn draw_sensor_trace(
    out: &mut String,
    movement: &MoveObservation,
    sensor: Sensor,
    y: f64,
    h: f64,
    x_for_time: &impl Fn(f64) -> f64,
) {
    let mut active = movement.start_sensors.get(sensor);
    let mut segment_start_s = movement.start_time_s;

    for edge in movement.edges.iter().filter(|edge| edge.sensor == sensor) {
        draw_sensor_segment(out, segment_start_s, edge.time_s, active, y, h, x_for_time);
        active = edge.active;
        segment_start_s = edge.time_s;
    }

    draw_sensor_segment(
        out,
        segment_start_s,
        movement.end_time_s,
        active,
        y,
        h,
        x_for_time,
    );
}

fn draw_sensor_segment(
    out: &mut String,
    start_time_s: f64,
    end_time_s: f64,
    active: bool,
    y: f64,
    h: f64,
    x_for_time: &impl Fn(f64) -> f64,
) {
    let x0 = x_for_time(start_time_s);
    let x1 = x_for_time(end_time_s);
    let fill = if active { "#fde68a" } else { "#e4e7eb" };
    write!(
        out,
        "<rect x=\"{x0:.2}\" y=\"{y:.2}\" width=\"{:.2}\" height=\"{h:.2}\" fill=\"{fill}\"/>",
        (x1 - x0).max(1.0),
    )
    .ok();
}

fn estimate_edge_position(edge_time_s: f64, telemetry: &[TelemetrySample]) -> Option<f64> {
    let mut before = None;
    let mut after = None;

    for sample in telemetry {
        if sample.dropped || sample.stale {
            continue;
        }
        let Some(position) = sample.position_mm else {
            continue;
        };
        if sample.time_s <= edge_time_s {
            before = Some((sample.time_s, position));
        } else {
            after = Some((sample.time_s, position));
            break;
        }
    }

    match (before, after) {
        (Some((t0, p0)), Some((t1, p1))) if t1 > t0 => {
            let fraction = (edge_time_s - t0) / (t1 - t0);
            Some(p0 + (p1 - p0) * fraction)
        }
        (Some((_, position)), None) | (None, Some((_, position))) => Some(position),
        _ => None,
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn fmt_error(_: std::fmt::Error) -> io::Error {
    io::Error::other("failed to format simulation report")
}

#[cfg(test)]
mod tests {
    use super::{html_report, run_simulation, SimConfig};
    use crate::homing::{Axis, HomingConfig};

    #[test]
    fn normal_start_positions_home_without_hard_stop_contact() {
        for start in super::default_starts() {
            let report = run_simulation(
                Axis::Z,
                *start,
                HomingConfig::default(),
                SimConfig::default(),
            );
            let result = report.result.expect("normal homing should succeed");
            assert!(!result.hard_stop_contact());
            assert!((result.home_datum_mm - 98.0).abs() < 0.02);
            assert!((result.final_position_mm - 96.0).abs() < 0.02);
            assert!(result.total_time_s().is_finite());
        }
    }

    #[test]
    fn missing_home_sensor_faults() {
        let mut sim_config = SimConfig::default();
        sim_config.home.missing = true;
        let report = run_simulation(Axis::X, 50.0, HomingConfig::default(), sim_config);
        assert!(report.result.is_err());
    }

    #[test]
    fn missing_coarse_sensor_faults() {
        let mut sim_config = SimConfig::default();
        sim_config.coarse.missing = true;
        let report = run_simulation(Axis::X, 50.0, HomingConfig::default(), sim_config);
        assert!(report.result.is_err());
    }

    #[test]
    fn inverted_home_sensor_faults() {
        let mut sim_config = SimConfig::default();
        sim_config.home.inverted = true;
        let report = run_simulation(Axis::X, 50.0, HomingConfig::default(), sim_config);
        assert!(report.result.is_err());
    }

    #[test]
    fn dropped_telemetry_does_not_break_homing() {
        let mut sim_config = SimConfig::default();
        sim_config.telemetry.drop_every = Some(7);
        let report = run_simulation(Axis::Z, 25.0, HomingConfig::default(), sim_config);
        let result = report
            .result
            .expect("dropped telemetry should be tolerated");
        assert!((result.home_datum_mm - 98.0).abs() < 0.05);
    }

    #[test]
    fn modestly_delayed_telemetry_does_not_break_homing() {
        let mut sim_config = SimConfig::default();
        sim_config.telemetry.delay_s = 0.020;
        let report = run_simulation(Axis::Z, 25.0, HomingConfig::default(), sim_config);
        let result = report
            .result
            .expect("delayed telemetry should be tolerated");
        assert!((result.home_datum_mm - 98.0).abs() < 0.05);
    }

    #[test]
    fn noisy_quantized_feedback_still_latches() {
        let mut sim_config = SimConfig::default();
        sim_config.telemetry.noise_mm = 0.005;
        sim_config.telemetry.quantize_mm = Some(0.01);
        sim_config.home.edge_jitter_mm = 0.01;
        let report = run_simulation(Axis::X, 84.0, HomingConfig::default(), sim_config);
        let result = report
            .result
            .expect("small deterministic imperfections should be tolerated");
        assert!((result.home_datum_mm - 98.0).abs() < 0.05);
    }

    #[test]
    fn html_report_contains_result_summary_and_svg() {
        let report = run_simulation(Axis::Z, 50.0, HomingConfig::default(), SimConfig::default());
        let html = html_report(&[report]);
        assert!(html.contains("<svg"));
        assert!(html.contains("Success"));
        assert!(html.contains("Commanded Moves"));
        assert!(html.contains("Offset mm"));
        assert!(html.contains("Rate mm/min"));
        assert!(html.contains("TCL125 Homing Simulation"));
    }
}
