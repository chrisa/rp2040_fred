fredctl (Host Bring-Up Tool)
============================

Status
- Phase A complete:
  - shared bridge packet protocol in firmware crate,
  - host CLI scaffold with command structure,
  - working `mock` monitor path for DRO decoding and display.
- `usb` transport path is currently a stub and will be implemented in the next phase.

Usage (mock mode)
- `cargo run --offline -- on mock`
- `cargo run --offline -- off mock`
- `cargo run --offline -- monitor mock 200`

Usage (usb mode, not implemented yet)
- `cargo run --offline -- on usb`
- `cargo run --offline -- off usb`
- `cargo run --offline -- monitor usb`

Notes
- X display uses diameter semantics (`x_counts * 2`) to match CNCMAN behavior.
- Z display uses direct axis counts.
- Conversion constants currently default to:
  - `x_counts_per_mm = 100`
  - `z_counts_per_mm = 100`
  and should be calibrated against real machine movement.
