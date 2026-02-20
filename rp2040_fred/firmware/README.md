RP2040 FRED Firmware Scaffold
=============================

Status
- USB bridge is always enabled and exposed over bulk endpoints.
- Shared bridge protocol logic lives in `../protocol` (`rp2040-fred-protocol`) so it can be tested on host targets.
- Transport feature flags are retained:
  - `mock-bus` (default): synthetic cadence-backed telemetry.
  - `pio-real`: reserved for live PIO transport bring-up.
- Uses `embassy-rp`.

Current Behavior
- `../protocol/src/bridge_proto.rs` defines host<->RP2040 packet framing and CRC32 checks.
- `../protocol/src/bridge_service.rs` handles bridge requests (`PING`, `TELEMETRY_SET`, `SNAPSHOT_REQ`) and emits telemetry events.
- `../protocol/src/dro_decode.rs` reconstructs X/Z/RPM from FC80/FCF1 command-response stream.
- `../protocol/src/protocol.rs` implements `FC80 -> (FCF0, FCF1)` logic for the DRO command cadence.
- `src/main.rs` runs USB bridge request/telemetry handling.
- `src/pins.rs` matches agreed shared-bus pin map.
- Bus words are composed as:
  - `[15:8] = A0..A7`
  - `[7:0]  = D0..D7`

Build
- Default mock check:
  - `cargo check`
- Real-transport scaffold check:
  - `cargo check --no-default-features --features pio-real,defmt-log`
- Firmware build:
  - `cargo fw-build-usb`
- Firmware flash+run over SWD (`probe-rs` runner):
  - `cargo fw-run-usb`
- Host-side protocol tests:
  - `cd ../protocol && cargo test`
- Host-side mock bus trace:
  - `cargo run --example mock_bus --target x86_64-unknown-linux-gnu -- 30`

Probe-rs Bring-Up
- Install CLI (if needed):
  - `cargo install probe-rs-tools --locked`
- Confirm probe visibility:
  - `probe-rs list`
- Runner is configured in `.cargo/config.toml` for RP2040:
  - `probe-rs run --chip RP2040 --connect-under-reset`
- Typical bring-up flow (Pico attached via SWD + USB):
  1. In `rp2040_fred/firmware`: `cargo fw-run-usb`
  2. In `rp2040_fred/host`: `cargo run -- monitor usb`

Next Wiring Tasks (`pio-real`)
1. Validate control-line polarity and bus turn-around timing on hardware captures.
2. Confirm command cadence against live lathe controller responses.
3. Add timeout/error counters around FIFO waits.
4. Timing tune against 1MHz bus with logic-analyzer captures.

Notes
- `pio-real` compiles and performs direct PIO FIFO transactions, but is not yet hardware-validated.
- FIFO waits in `pio-real` are bounded with timeout counters to avoid hard lockups during bring-up.
