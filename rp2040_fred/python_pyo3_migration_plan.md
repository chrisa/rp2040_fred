# Python/Rust PyO3 Migration Plan

## Goal

Make the Python client use the Rust implementation for USB transport and telemetry decoding, while keeping the external Python `FredUsbClient` API stable for monitoring use cases.

Scope boundary:

- Python only needs `monitor`/telemetry snapshots.
- Python does **not** need raw passive capture consumption.
- The Rust CLI in `host/` should keep working.

## Current duplication

Python currently duplicates Rust logic in two critical areas:

- USB discovery, interface claiming, bulk read/write, timeout handling, and request/ACK matching in [python/fred_client.py](/home/chris/Projects/tcl125/tcl202_dis/rp2040_fred/python/fred_client.py) and [host/src/transport.rs](/home/chris/Projects/tcl125/tcl202_dis/rp2040_fred/host/src/transport.rs).
- Bridge packet framing and telemetry decoding in [python/fred_client.py](/home/chris/Projects/tcl125/tcl202_dis/rp2040_fred/python/fred_client.py), [protocol/src/bridge_proto.rs](/home/chris/Projects/tcl125/tcl202_dis/rp2040_fred/protocol/src/bridge_proto.rs), and [protocol/src/trace_decode.rs](/home/chris/Projects/tcl125/tcl202_dis/rp2040_fred/protocol/src/trace_decode.rs).

That duplication is the right thing to remove first. Python trace/capture support can be dropped rather than ported.

## Recommended architecture

Use three layers:

1. `protocol/`
   Shared packet and DRO conversion code. Keep this as the canonical protocol/decoder crate.
2. `host/`
   Turn this into a real reusable Rust library for USB monitor operations, with the CLI in `host/src/main.rs` calling the library instead of owning the logic directly.
3. PyO3 extension crate
   Add a small Python bindings crate which depends on the `host` library and exposes Python-facing classes/functions.

This keeps the Rust USB code in one place. The PyO3 crate should be thin glue, not the home of transport logic.

## Proposed Rust refactor

### 1. Promote `host/` to a reusable library

Move the reusable pieces out of the binary path:

- Make `host/src/lib.rs` export:
  - `UsbTransport`
  - a monitor-focused client type, for example `FredMonitorClient`
  - a plain Rust snapshot type, for example `MonitorSnapshot`
  - Rust error types suitable for both CLI and PyO3 conversion
- Keep `host/src/main.rs` as a thin CLI wrapper over library calls.

### 2. Define a monitor-only Rust API

Keep the Rust API narrower than the full CLI:

- `open(vid, pid, timeout_ms, calibration) -> FredMonitorClient`
- `enable_polling(period_ms)`
- `disable_polling()`
- `refresh() -> MonitorSnapshot`
- `close()`

`MonitorSnapshot` should contain exactly what Python already returns:

- `x_mm`
- `z_mm`
- `spindle_rpm`
- `x_counts`
- `z_counts`
- `tick`
- `flags`

Do not expose trace sample decoding or capture streaming in this layer unless the CLI needs it separately.

### 3. Remove Python-specific protocol logic

Once the Rust monitor API exists, Python should no longer own:

- packet encode/decode
- CRC validation
- USB endpoint discovery/claim/release
- request/reply sequencing
- telemetry payload parsing

## PyO3 design

### Binding shape

Expose a native module with a Python class that mirrors the current API closely:

- `FredUsbClient.__init__(vid, pid, timeout_ms=250, x_counts_per_mm=100.0, z_counts_per_mm=100.0)`
- `enable_polling(period_ms=25)`
- `disable_polling()`
- `refresh() -> dict`
- `close()`

Recommended implementation:

- Rust `#[pyclass]` wrapping `FredMonitorClient`
- Rust `#[pymethods]` for the methods above
- `refresh()` returning a Python `dict` or a small `@dataclass`-like Python object converted back to the existing dict shape

### Compatibility wrapper

Keep [python/fred_client.py](/home/chris/Projects/tcl125/tcl202_dis/rp2040_fred/python/fred_client.py), but reduce it to a compatibility shim:

- import the native extension
- re-export `FredUsbClient`
- keep any small Python-only conveniences if needed

This preserves import paths for existing callers.

### Capture methods

There is one API decision to make:

- safest compatibility path: keep `enable_capture`, `disable_capture`, and `read_capture_samples`, but raise `NotImplementedError`
- stricter cleanup path: remove them and accept a Python breaking change

Given the stated requirement to retain the external `FredClient` API, the first option is safer.

## Packaging/build plan

Use a mixed Python/Rust package with `maturin`.

Recommended layout:

- `python/pyproject.toml`
- `python/fred_client.py`
- `python/monitor_loop.py`
- `python/rust_ext/Cargo.toml`
- `python/rust_ext/src/lib.rs`

The bindings crate should depend on:

- `pyo3`
- the reusable `host` crate
- `rp2040-fred-protocol` only if the binding needs direct protocol types

Build expectations:

- development install via `maturin develop`
- wheel build via `maturin build`

Notes from current docs:

- `maturin` auto-detects `pyo3` bindings from `Cargo.toml`
- `abi3` is available if we want stable wheels across Python versions
- PyO3 workspace/test setups still commonly make `extension-module` an optional feature, enabled by `maturin`, to avoid linker/test friction

## Implementation phases

### Phase 1: Rust library extraction

- Refactor `host/` so USB transport and monitor logic live in `host/src/lib.rs`.
- Keep CLI behavior unchanged.
- Add Rust unit tests around monitor packet handling and snapshot conversion.

Exit criteria:

- `cargo test --manifest-path host/Cargo.toml`
- CLI still supports `monitor usb`

### Phase 2: PyO3 extension

- Create the bindings crate under `python/`.
- Expose `FredUsbClient` with the monitor methods only.
- Convert Rust errors into Python exceptions.
- Make blocking USB reads release the GIL where appropriate.

Exit criteria:

- `python -c "from fred_client import FredUsbClient"`
- `monitor_loop.py` runs against the native backend

### Phase 3: Python compatibility shim

- Replace the existing pure-Python USB implementation with a thin wrapper.
- Keep return values and method signatures aligned with current callers.
- Stub unsupported capture methods explicitly.

Exit criteria:

- Existing monitor callers do not need code changes
- No `pyusb` dependency remains for normal use

### Phase 4: Cleanup

- Remove obsolete Python packet/USB decoder code.
- Update `python/README.md` with native build/install instructions.
- Add a short developer note describing the Rust/Python boundary.

## Testing plan

Rust:

- unit tests for packet/telemetry translation at the library boundary
- tests for ACK/NACK handling and timeout mapping where practical

Python:

- import test for the extension module
- API compatibility tests for `FredUsbClient`
- smoke test for `refresh()` shape and numeric field names

Hardware/manual:

- verify `enable_polling(25)` still disables capture first if that firmware assumption remains required
- verify `refresh()` drains telemetry and returns the latest snapshot
- verify clean shutdown and repeated open/close cycles

## Risks and decisions

### 1. Crate structure

Risk:
`host/` is currently a binary-first crate, so some refactoring is unavoidable.

Decision:
Prefer extracting a real host library and keeping the PyO3 crate thin. Do not embed the USB logic directly into the bindings crate.

### 2. Python API compatibility

Risk:
Dropping capture methods outright may break existing imports even if callers do not use them.

Decision:
Preserve method names where practical and fail explicitly for unsupported capture calls.

### 3. Threading/GIL behavior

Risk:
USB calls may block long enough to make Python integration feel poor if they hold the GIL.

Decision:
Wrap blocking Rust I/O in the PyO3 pattern that releases the GIL during the call.

### 4. Build ergonomics

Risk:
PyO3 packaging can become awkward if the extension crate and normal Cargo testing are tightly coupled.

Decision:
Use `maturin` for Python packaging and keep any `extension-module` feature optional if linker friction appears in workspace tests.

## Suggested first implementation slice

Start with the smallest vertical slice that proves the design:

1. Extract a Rust `FredMonitorClient` from the current host USB path.
2. Add a PyO3 `FredUsbClient` exposing `enable_polling`, `disable_polling`, `refresh`, and `close`.
3. Replace Python `monitor_loop.py` to use the native-backed import.
4. Leave capture methods present but unsupported.

If that slice works on hardware, the remaining migration is mostly cleanup.

## Reference material checked

- PyO3 user guide on Python modules: https://pyo3.rs/v0.19.2/module
- Maturin bindings guide: https://www.maturin.rs/bindings.html
- PyO3 FAQ note about making `extension-module` optional in test/workspace setups: https://pyo3.rs/v0.22.3/faq
