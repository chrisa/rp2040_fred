fredctl (Host Bring-Up Tool)
============================

Status
- Phase A complete:
  - shared bridge packet protocol in `rp2040-fred-protocol`,
  - firmware-side bridge service with request handling and telemetry event generation,
  - host CLI scaffold with command structure,
  - working `mock` monitor path that consumes real bridge `TELEMETRY` packets.
- `usb` transport is implemented with `rusb` bulk IN/OUT endpoint access.

Usage (mock mode)
- `cargo run --offline -- on mock`
- `cargo run --offline -- off mock`
- `cargo run --offline -- monitor mock 200`

Usage (usb mode)
- `cargo run --offline -- on usb`
- `cargo run --offline -- off usb`
- `cargo run --offline -- monitor usb`
- `cargo run --offline -- capture-on usb`
- `cargo run --offline -- capture-off usb`
- `cargo run --offline -- capture usb`

Notes
- X display uses diameter semantics (`x_counts * 2`) to match CNCMAN behavior.
- Z display uses direct axis counts.
- Mock telemetry emits one packet per full 10-command DRO cadence.
- Default USB target is `VID=0x2E8A`, `PID=0x000A`, with the first bulk IN/OUT interface discovered at runtime.
- Firmware now powers up in passive capture mode; `monitor usb` automatically disables capture and enables DRO telemetry.
- Conversion constants currently default to:
  - `x_counts_per_mm = 100`
  - `z_counts_per_mm = 100`
  and should be calibrated against real machine movement.
