use std::io;
use std::time::Duration;

use fredctl::monitor::{Calibration, FeedbackTimingSequence, FredMonitorClient, MonitorSnapshot};
use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use rp2040_fred_protocol::bridge_proto::{
    ControllerStatus, ExperimentBusOp, ExperimentBusOpKind, ExperimentRecord, ExperimentStatus,
    RpmServiceMode, EXPERIMENT_STATUS_ACTIVE, EXPERIMENT_STATUS_DONE, EXPERIMENT_STATUS_ERROR,
    EXPERIMENT_STATUS_RECORDS_DROPPED,
};

create_exception!(_fred_native, FredProtocolError, PyRuntimeError);
create_exception!(_fred_native, FredUsbError, PyRuntimeError);

#[pyclass(unsendable)]
struct FredUsbClient {
    inner: Option<FredMonitorClient>,
}

#[pymethods]
impl FredUsbClient {
    #[new]
    #[pyo3(signature = (vid, pid, *, timeout_ms=1000, x_counts_per_mm=100.0, z_counts_per_mm=100.0))]
    fn new(
        vid: u16,
        pid: u16,
        timeout_ms: u64,
        x_counts_per_mm: f32,
        z_counts_per_mm: f32,
    ) -> PyResult<Self> {
        let calibration = Calibration {
            x_counts_per_mm,
            z_counts_per_mm,
        };
        let inner = FredMonitorClient::open_with_options(
            vid,
            pid,
            Duration::from_millis(timeout_ms),
            calibration,
        )
        .map_err(map_io_error)?;
        Ok(Self { inner: Some(inner) })
    }

    #[pyo3(signature = (period_ms=25, rpm_service="manual"))]
    fn enable_polling(
        &mut self,
        py: Python<'_>,
        period_ms: u16,
        rpm_service: &str,
    ) -> PyResult<()> {
        let rpm_service_mode = parse_rpm_service_mode(rpm_service)?;
        self.with_client(py, move |client| {
            client.enable_polling(period_ms, rpm_service_mode)
        })
    }

    fn disable_polling(&mut self, py: Python<'_>) -> PyResult<()> {
        self.with_client(py, FredMonitorClient::disable_polling)
    }

    fn refresh<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let snapshot = self.with_client(py, FredMonitorClient::next_snapshot)?;
        snapshot_to_dict(py, snapshot)
    }

    #[pyo3(signature = (timeout_ms=0))]
    fn refresh_timeout<'py>(
        &mut self,
        py: Python<'py>,
        timeout_ms: u64,
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        let snapshot = self.with_client(py, move |client| {
            client.refresh_timeout(Duration::from_millis(timeout_ms))
        })?;
        snapshot
            .map(|snapshot| snapshot_to_dict(py, snapshot))
            .transpose()
    }

    fn next_snapshot<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let snapshot = self.with_client(py, FredMonitorClient::next_snapshot)?;
        snapshot_to_dict(py, snapshot)
    }

    fn latest_snapshot<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let snapshot = self.with_client(py, |client| Ok(client.latest()))?;
        snapshot
            .map(|snapshot| snapshot_to_dict(py, snapshot))
            .transpose()
    }

    #[pyo3(signature = (*, x_mm=0.0, z_mm=0.0, slew=61, wait=false))]
    fn rapid_move_delta(
        &mut self,
        py: Python<'_>,
        x_mm: f32,
        z_mm: f32,
        slew: u16,
        wait: bool,
    ) -> PyResult<bool> {
        self.with_client(py, move |client| {
            client.rapid_move_delta_mm(x_mm, z_mm, slew, wait)
        })
    }

    #[pyo3(signature = (*, x_mm=0.0, z_mm=0.0, feed=100, slew=61, wait=false))]
    fn feed_move_delta(
        &mut self,
        py: Python<'_>,
        x_mm: f32,
        z_mm: f32,
        feed: u32,
        slew: u16,
        wait: bool,
    ) -> PyResult<bool> {
        self.with_client(py, move |client| {
            client.feed_move_delta_mm(x_mm, z_mm, feed, slew, wait)
        })
    }

    fn controller_status<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let status = self.with_client(py, FredMonitorClient::controller_status)?;
        status_to_dict(py, status)
    }

    #[pyo3(signature = (*, x_mm=0.0, z_mm=0.0, mode="rapid", feed=100, slew=61, feedback_period_ms=10, trial_id=0, script_ops=None, feedback_timing=false))]
    fn run_experiment_move_delta(
        &mut self,
        py: Python<'_>,
        x_mm: f32,
        z_mm: f32,
        mode: &str,
        feed: u32,
        slew: u16,
        feedback_period_ms: u16,
        trial_id: u32,
        script_ops: Option<Vec<(u8, u8, u8, u8, u8, u32)>>,
        feedback_timing: bool,
    ) -> PyResult<bool> {
        let feed = match mode {
            "rapid" => None,
            "feed" => Some(feed),
            _ => {
                return Err(FredProtocolError::new_err(format!(
                    "unknown experiment move mode: {mode}"
                )));
            }
        };
        let script = parse_script_ops(script_ops.unwrap_or_default())?;
        self.with_client(py, move |client| {
            client.run_experiment_move_delta_mm(
                x_mm,
                z_mm,
                feed,
                slew,
                feedback_period_ms,
                trial_id,
                &script,
                feedback_timing,
            )
        })
    }

    #[pyo3(signature = (*, feedback_period_ms=10, trial_id=0, poll_count=30, sequence="full"))]
    fn run_feedback_timing_experiment(
        &mut self,
        py: Python<'_>,
        feedback_period_ms: u16,
        trial_id: u32,
        poll_count: u32,
        sequence: &str,
    ) -> PyResult<()> {
        let sequence = parse_feedback_timing_sequence(sequence)?;
        self.with_client(py, move |client| {
            client.run_feedback_timing_experiment(
                feedback_period_ms,
                trial_id,
                poll_count,
                sequence,
            )
        })
    }

    #[pyo3(signature = (timeout_ms=0))]
    fn next_experiment_record<'py>(
        &mut self,
        py: Python<'py>,
        timeout_ms: u64,
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        let record = self.with_client(py, move |client| {
            client.next_experiment_record_timeout(Duration::from_millis(timeout_ms))
        })?;
        record
            .map(|record| experiment_record_to_dict(py, record))
            .transpose()
    }

    fn experiment_status<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let status = self.with_client(py, FredMonitorClient::experiment_status)?;
        experiment_status_to_dict(py, status)
    }

    #[pyo3(signature = (timeout_ms=None))]
    fn wait_experiment_idle(&mut self, py: Python<'_>, timeout_ms: Option<u64>) -> PyResult<()> {
        self.with_client(py, move |client| {
            client.wait_experiment_idle(timeout_ms.map(Duration::from_millis))
        })
    }

    #[pyo3(signature = (timeout_ms=None))]
    fn wait_idle(&mut self, py: Python<'_>, timeout_ms: Option<u64>) -> PyResult<()> {
        self.with_client(py, move |client| {
            client.wait_idle(timeout_ms.map(Duration::from_millis))
        })
    }

    #[pyo3(signature = (*, on, rpm=0.0, forward=true, ssl=None, wait=false))]
    fn set_spindle(
        &mut self,
        py: Python<'_>,
        on: bool,
        rpm: f32,
        forward: bool,
        ssl: Option<u16>,
        wait: bool,
    ) -> PyResult<bool> {
        self.with_client(py, move |client| {
            client.set_spindle(on, rpm, forward, ssl, wait)
        })
    }

    #[pyo3(signature = (*, current_station, target_station, slew=61, wait=false))]
    fn change_tool(
        &mut self,
        py: Python<'_>,
        current_station: u8,
        target_station: u8,
        slew: u16,
        wait: bool,
    ) -> PyResult<bool> {
        self.with_client(py, move |client| {
            client.change_tool(current_station, target_station, slew, wait)
        })
    }

    fn close(&mut self, py: Python<'_>) {
        if let Some(client) = self.inner.take() {
            py.detach(move || client.close());
        }
    }

    fn __enter__(slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        slf
    }

    #[pyo3(signature = (_exc_type=None, _exc=None, _tb=None))]
    fn __exit__(
        &mut self,
        py: Python<'_>,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc: Option<&Bound<'_, PyAny>>,
        _tb: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        self.close(py);
        false
    }
}

impl FredUsbClient {
    fn with_client<T: Send>(
        &mut self,
        py: Python<'_>,
        f: impl FnOnce(&mut FredMonitorClient) -> io::Result<T> + Send,
    ) -> PyResult<T> {
        let client = self
            .inner
            .as_mut()
            .ok_or_else(|| FredUsbError::new_err("device not open"))?;
        py.detach(|| f(client)).map_err(map_io_error)
    }
}

fn snapshot_to_dict<'py>(
    py: Python<'py>,
    snapshot: MonitorSnapshot,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("x_mm", snapshot.x_mm)?;
    dict.set_item("z_mm", snapshot.z_mm)?;
    dict.set_item("spindle_rpm", snapshot.spindle_rpm)?;
    dict.set_item("x_counts", snapshot.x_counts)?;
    dict.set_item("z_counts", snapshot.z_counts)?;
    dict.set_item("tick", snapshot.tick)?;
    dict.set_item("generation", snapshot.generation)?;
    dict.set_item("flags", snapshot.flags)?;
    Ok(dict)
}

fn status_to_dict<'py>(py: Python<'py>, status: ControllerStatus) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("flags", status.flags)?;
    dict.set_item("pending_count", status.pending_count)?;
    dict.set_item("idle", status.is_idle())?;
    dict.set_item("error", status.has_error())?;
    Ok(dict)
}

fn experiment_status_to_dict<'py>(
    py: Python<'py>,
    status: ExperimentStatus,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("flags", status.flags)?;
    dict.set_item("pending_records", status.pending_records)?;
    dict.set_item("dropped_records", status.dropped_records)?;
    dict.set_item("active_trial_id", status.active_trial_id)?;
    dict.set_item("active", status.flags & EXPERIMENT_STATUS_ACTIVE != 0)?;
    dict.set_item("done", status.flags & EXPERIMENT_STATUS_DONE != 0)?;
    dict.set_item("error", status.flags & EXPERIMENT_STATUS_ERROR != 0)?;
    dict.set_item(
        "records_dropped",
        status.flags & EXPERIMENT_STATUS_RECORDS_DROPPED != 0,
    )?;
    Ok(dict)
}

fn experiment_record_to_dict<'py>(
    py: Python<'py>,
    record: ExperimentRecord,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    match record {
        ExperimentRecord::Sample(record) => {
            dict.set_item("kind", "sample")?;
            dict.set_item("trial_id", record.trial_id)?;
            dict.set_item("timestamp_us", record.timestamp_us)?;
            dict.set_item("sample_index", record.sample_index)?;
            dict.set_item("x_counts", record.x_counts)?;
            dict.set_item("z_counts", record.z_counts)?;
            dict.set_item("spindle_rpm", record.rpm)?;
            dict.set_item("flags", record.flags)?;
        }
        ExperimentRecord::BusOp(record) => {
            dict.set_item("kind", "bus_op")?;
            dict.set_item("trial_id", record.trial_id)?;
            dict.set_item("timestamp_us", record.timestamp_us)?;
            dict.set_item("op_index", record.op_index)?;
            dict.set_item("op_kind", record.op_kind as u8)?;
            dict.set_item("status", record.status)?;
            dict.set_item("addr", record.addr)?;
            dict.set_item("write_value", record.write_value)?;
            dict.set_item("read_value", record.read_value)?;
        }
        ExperimentRecord::Event(record) => {
            dict.set_item("kind", "event")?;
            dict.set_item("trial_id", record.trial_id)?;
            dict.set_item("timestamp_us", record.timestamp_us)?;
            dict.set_item("event", record.event as u8)?;
            dict.set_item("status", record.status)?;
            dict.set_item("flags", record.flags)?;
        }
        ExperimentRecord::FeedbackTiming(record) => {
            dict.set_item("kind", "feedback_timing")?;
            dict.set_item("trial_id", record.trial_id)?;
            dict.set_item("timestamp_us", record.timestamp_us)?;
            dict.set_item("poll_index", record.poll_index)?;
            dict.set_item("cmd_index", record.cmd_index)?;
            dict.set_item("cmd", record.cmd)?;
            dict.set_item("value", record.value)?;
            dict.set_item("flags", record.flags)?;
            dict.set_item("total_us", record.total_us)?;
            dict.set_item("wait_before_us", record.wait_before_us)?;
            dict.set_item("wait_after_us", record.wait_after_us)?;
            dict.set_item("reads_before", record.reads_before)?;
            dict.set_item("reads_after", record.reads_after)?;
        }
    }
    Ok(dict)
}

fn parse_script_ops(ops: Vec<(u8, u8, u8, u8, u8, u32)>) -> PyResult<Vec<ExperimentBusOp>> {
    let mut out = Vec::with_capacity(ops.len());
    for (kind, addr, value, mask, match_value, arg_us) in ops {
        out.push(ExperimentBusOp {
            kind: ExperimentBusOpKind::from_u8(kind).ok_or_else(|| {
                FredProtocolError::new_err(format!("unknown experiment bus op kind: {kind}"))
            })?,
            addr,
            value,
            mask,
            match_value,
            arg_us,
        });
    }
    Ok(out)
}

fn parse_rpm_service_mode(value: &str) -> PyResult<RpmServiceMode> {
    match value {
        "manual" | "fc88" | "88" => Ok(RpmServiceMode::Manual),
        "remote" | "fcad" | "ad" => Ok(RpmServiceMode::Remote),
        _ => Err(FredProtocolError::new_err(format!(
            "unknown RPM service mode: {value}"
        ))),
    }
}

fn parse_feedback_timing_sequence(value: &str) -> PyResult<FeedbackTimingSequence> {
    match value {
        "full" | "xzrpm" => Ok(FeedbackTimingSequence::Full),
        "xz" | "axes" => Ok(FeedbackTimingSequence::Xz),
        "x" => Ok(FeedbackTimingSequence::X),
        "z" => Ok(FeedbackTimingSequence::Z),
        _ => Err(FredProtocolError::new_err(format!(
            "unknown feedback timing sequence: {value}"
        ))),
    }
}

fn map_io_error(err: io::Error) -> PyErr {
    let message = err.to_string();
    match err.kind() {
        io::ErrorKind::InvalidData => FredProtocolError::new_err(message),
        _ => FredUsbError::new_err(message),
    }
}

#[pymodule]
mod _fred_native {
    #[pymodule_export]
    use super::FredUsbError;

    #[pymodule_export]
    use super::FredProtocolError;

    #[pymodule_export]
    use super::FredUsbClient;
}
