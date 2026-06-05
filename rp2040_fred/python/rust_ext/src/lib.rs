use std::io;
use std::time::Duration;

use fredctl::monitor::{Calibration, FredMonitorClient, MonitorSnapshot};
use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use rp2040_fred_protocol::bridge_proto::{ControllerStatus, RpmServiceMode};

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

fn parse_rpm_service_mode(value: &str) -> PyResult<RpmServiceMode> {
    match value {
        "manual" | "fc88" | "88" => Ok(RpmServiceMode::Manual),
        "remote" | "fcad" | "ad" => Ok(RpmServiceMode::Remote),
        _ => Err(FredProtocolError::new_err(format!(
            "unknown RPM service mode: {value}"
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
