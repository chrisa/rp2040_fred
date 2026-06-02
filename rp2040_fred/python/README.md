# Python Monitor Client

The Python client is now a compatibility wrapper over a Rust implementation
built with `pyo3`.

## Status

- Monitoring/telemetry is supported.
- Immediate jog-style rapid/feed deltas are supported for the LinuxCNC
  userspace component.
- Spindle start/stop command output is supported through the same master USB
  command path.
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
client.enable_polling(period_ms=10, rpm_service="manual")

snapshot = client.next_snapshot()
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

`refresh(timeout_ms=0)` is the LinuxCNC-friendly polling call.  It drains any
available telemetry and returns the newest snapshot from that drain, or `None`
if no new telemetry arrived before the timeout.  `latest_snapshot()` returns the
last decoded telemetry snapshot without blocking.

`enable_polling()` takes `rpm_service="manual"` or `"remote"`.  Use
`"manual"` for monitor-style scripts while the lathe is in manual mode; it uses
the `FC88` RPM trigger.  Use `"remote"` when the bridge is controlling motion or
spindle commands; it uses the old manufacturing host's `FCAD` speed-service
write.

Jog deltas are exposed in machine units:

```python
client.rapid_move_delta(x_mm=0.1, z_mm=0.0, slew=61, wait=False)
client.feed_move_delta(x_mm=0.0, z_mm=-0.1, feed=100, slew=61, wait=False)
client.set_spindle(on=True, rpm=3000.0, forward=False, wait=False)
client.set_spindle(on=False, wait=False)
status = client.controller_status()
```

X deltas use LinuxCNC/display diameter semantics.  The Rust layer converts them
to the controller's radius-count payload and rounds to an even diameter count.
Z deltas are direct axis deltas.

Spindle start uses the captured `do(0,3/4,0,0,0,0,0,0,SSL,0)` command family:
`forward=True` emits the inferred forward subcode `4`, while `forward=False`
emits the captured reverse subcode `3`.  RPM is converted to the old host's
`SSL ~= rpm / 24` field and capped at 127.  For direct probing, pass
`ssl=<0..127>` to bypass RPM conversion.

## Unsupported capture API

The compatibility layer keeps these methods so existing imports fail
explicitly rather than silently changing behavior:

- `enable_capture()`
- `disable_capture()`
- `read_capture_samples()`

Each raises `NotImplementedError`.
