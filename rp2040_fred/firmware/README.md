RP2040 FRED Firmware Scaffold
=============================

Status
- DRO protocol core is implemented for command cadence and synthetic telemetry.
- Transport supports two build modes:
  - `mock-bus` (default): no hardware access, protocol/loop bring-up only.
  - `pio-real`: loads and starts real PIO state machines for the shared D0..D7 bus model.
- `usb-bridge` feature is present but intentionally blocked at compile-time until Embassy crate versions are aligned (`embassy-rp` and `embassy-usb` currently pull incompatible `embassy-usb-driver` majors in this workspace).
- Uses `embassy-rp`.

Current Behavior
- `src/bridge_proto.rs` defines host<->RP2040 packet framing and CRC32 checks.
- `src/bridge_service.rs` handles bridge requests (`PING`, `TELEMETRY_SET`, `SNAPSHOT_REQ`) and emits telemetry events.
- `src/dro_decode.rs` reconstructs X/Z/RPM from FC80/FCF1 command-response stream.
- `src/protocol.rs` implements `FC80 -> (FCF0, FCF1)` logic for:
  - `03,02,01,00` (X sign + digits)
  - `07,06,05,04` (Z sign + digits)
  - `0D,0C` (speed digits path)
- `src/main.rs` runs a 10-command telemetry cadence.
- `src/pins.rs` matches agreed shared-bus pin map.
- Bus words are composed as:
  - `[15:8] = A0..A7`
  - `[7:0]  = D0..D7`

Build
- Default mock check:
  - `cargo check`
- Real-transport scaffold check:
  - `cargo check --no-default-features --features pio-real`
- Host-side protocol tests:
  - `cargo test --lib --target x86_64-unknown-linux-gnu`
- Host-side mock bus trace:
  - `cargo run --example mock_bus --target x86_64-unknown-linux-gnu -- 30`

Next Wiring Tasks (`pio-real`)
1. Validate control-line polarity and bus turn-around timing on hardware captures.
2. Confirm command cadence against live lathe controller responses.
3. Add timeout/error counters around FIFO waits.
4. Timing tune against 1MHz bus with logic-analyzer captures.

Notes
- `pio-real` compiles and performs direct PIO FIFO transactions, but is not yet hardware-validated.
- FIFO waits in `pio-real` are bounded with timeout counters to avoid hard lockups during bring-up.
