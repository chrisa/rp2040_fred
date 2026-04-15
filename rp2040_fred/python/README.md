# Python Monitor Client

The Python client is now a compatibility wrapper over a Rust implementation
built with `pyo3`.

## Status

- Monitoring/telemetry is supported.
- Passive capture is intentionally not exposed in Python.
- The external import path remains `from fred_client import FredUsbClient`.

## Development install

Create a virtual environment and install `maturin`:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip maturin
```

Then build an editable local install from the `python/` directory:

```bash
maturin develop
```

## Usage

```python
from fred_client import FredUsbClient

client = FredUsbClient(vid=0x2E8A, pid=0x000A)
client.enable_polling(period_ms=25)

snapshot = client.refresh()
print(snapshot)
# {
#   "x_mm": ...,
#   "z_mm": ...,
#   "spindle_rpm": ...,
#   "x_counts": ...,
#   "z_counts": ...,
#   "tick": ...,
#   "flags": ...,
# }

client.disable_polling()
client.close()
```

## Unsupported capture API

The compatibility layer keeps these methods so existing imports fail
explicitly rather than silently changing behavior:

- `enable_capture()`
- `disable_capture()`
- `read_capture_samples()`

Each raises `NotImplementedError`.
