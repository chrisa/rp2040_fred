RP2040 FRED Firmware Scaffold
=============================

Status
- USB bridge is always enabled and exposed over bulk endpoints.
- Shared bridge protocol logic lives in `../protocol` (`rp2040-fred-protocol`) so it can be tested on host targets.
- Transport feature flags are retained:
  - `mock-bus` (default): protocol bring-up with synthetic cadence-backed telemetry.
  - `pio-real`: passive PIO bus sniffer path.
- Uses `embassy-rp`.

Current Behavior
- `../protocol/src/bridge_proto.rs` defines host<->RP2040 packet framing and CRC32 checks.
- `../protocol/src/bridge_service.rs` handles DRO requests (`PING`, `TELEMETRY_SET`, `SNAPSHOT_REQ`) and emits telemetry events (mock path).
- `../protocol/src/dro_decode.rs` reconstructs X/Z/RPM from FC80/FCF1 command-response stream.
- `../protocol/src/protocol.rs` implements `FC80 -> (FCF0, FCF1)` logic for the DRO command cadence.
- `src/main.rs` runs USB packet IO and delegates transport behavior.
- `src/transport_mock.rs` handles mock bridge requests/events.
- `src/transport_pio.rs` handles passive PIO capture requests/events and `TRACE_SAMPLE` streaming.
- `../pio/passive_sniffer.pio` captures GPIO[20:0] on each 1MHZE edge (rising and falling) while `FRED_N` is asserted.
- `CAPTURE_SET` controls mode:
  - enabled (`1`): passive trace streaming.
  - disabled (`0`): non-capture request handling (mock telemetry path today).

Build
- Default mock check:
  - `cargo check`
- Real-transport scaffold check:
  - `cargo check --no-default-features --features pio-real,defmt-log`
- Firmware build:
  - `cargo fw-build`
- Firmware flash+run over SWD (`probe-rs` runner):
  - `cargo fw-run`
- Passive sniffer build/run:
  - `cargo fw-build-pio`
  - `cargo fw-run-pio`
- Host-side protocol tests:
  - `cd ../protocol && cargo test`

Probe-rs Bring-Up
- Install CLI (if needed):
  - `cargo install probe-rs-tools --locked`
- Confirm probe visibility:
  - `probe-rs list`
- Runner is configured in `.cargo/config.toml` for RP2040:
  - `probe-rs run --chip RP2040 --probe 2e8a:000c`
- Typical bring-up flow (Pico attached via SWD + USB):
  1. In `rp2040_fred/firmware`: `cargo fw-run-pio`
  2. In `rp2040_fred/host`: `cargo run -- capture usb`

Next Wiring Tasks (`pio-real`)
1. Confirm sampled bit mapping against logic analyzer captures.
2. Verify sustained capture throughput for expected FRED transaction bursts.
3. Add host-side binary trace logging for offline decode/timing analysis.
4. Add active bus-master mode back in once pin rework is complete.

Notes
- `pio-real` is currently passive-only; it does not drive the external bus.
