use std::io;
use std::time::Duration;

use fredctl::monitor::{FredMonitorClient, MonitorSnapshot};
use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use rp2040_fred_protocol::trace_decode::Calibration;

create_exception!(_fred_native, FredProtocolError, PyRuntimeError);
create_exception!(_fred_native, FredUsbError, PyRuntimeError);

#[pyclass(unsendable)]
struct FredUsbClient {
    inner: Option<FredMonitorClient>,
}

#[pymethods]
impl FredUsbClient {
    #[new]
    #[pyo3(signature = (vid, pid, *, timeout_ms=250, x_counts_per_mm=100.0, z_counts_per_mm=100.0))]
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

    #[pyo3(signature = (period_ms=25))]
    fn enable_polling(&mut self, py: Python<'_>, period_ms: u16) -> PyResult<()> {
        self.with_client(py, |client| client.enable_polling(period_ms))
    }

    fn disable_polling(&mut self, py: Python<'_>) -> PyResult<()> {
        self.with_client(py, FredMonitorClient::disable_polling)
    }

    fn refresh<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let snapshot = self.with_client(py, FredMonitorClient::refresh)?;
        snapshot_to_dict(py, snapshot)
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
    dict.set_item("flags", snapshot.flags)?;
    Ok(dict)
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