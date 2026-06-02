RP2040 FRED Firmware Scaffold
=============================

Status
- USB bridge is always enabled and exposes two vendor-specific bulk interfaces.
- Shared bridge protocol logic lives in `../protocol` (`rp2040-fred-protocol`) so it can be tested on host targets.
- Uses `embassy-rp`.

Current Behavior
- `../protocol/src/bridge_proto.rs` defines host<->RP2040 packet framing and CRC32 checks.
- `src/main.rs` runs USB packet IO for the master and capture interfaces.
- `src/main.rs` selects `Passive` or `Master` with the startup-time `TRANSPORT_MODE` constant; both transports are built into the firmware.
- `src/usb_bulk.rs` builds the two vendor-specific bulk interfaces without CMSIS-DAP.
- `src/transport/master.rs` handles active bus-master operation, queued controller work, telemetry, and passive trace streaming.
- `src/transport/passive.rs` keeps the standalone passive sniffer path for passive capture/telemetry without bus-mastering.
- The master USB interface uses protocol `0x01` for `PING`, `TELEMETRY_SET`, `COMMAND_BLOCK`, `CONTROLLER_ACTION`, and `CONTROLLER_STATUS_REQ`.
- The capture USB interface uses protocol `0x02` for `PING`, `CAPTURE_SET`, and `TRACE_SAMPLE` streaming.
- `src/transport/pio/passive.pio` captures GPIO0..17 on each 1MHZE edge while GPIO20/`FRED_N` is asserted.
- Wiring matches the `non-consec` branch:
  - `GPIO0..7 = D0..D7`
  - `GPIO8..15 = A0..A7`
  - `GPIO16 = RnW`
  - `GPIO17 = 1MHZE`
  - `GPIO20 = FRED_N`
  - `GPIO27 = DATA_DIR`
  - `GPIO28 = DATA_OE_N`
- `CAPTURE_SET` controls only the capture stream; it does not block master motion or telemetry commands.
- In passive startup mode, the same USB interfaces are exposed, but motion/controller commands are rejected because the selected transport is passive-only.

Build
- Firmware build:
  - `cargo fw-build`
- Firmware flash+run over SWD (`probe-rs` runner):
  - `cargo fw-run`
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
  1. In `rp2040_fred/firmware`: `cargo fw-run`
  2. In `rp2040_fred/host`: `cargo run -- capture usb`
  3. In another `rp2040_fred/host` shell: run a master command such as `cargo run -- monitor usb` or `cargo run -- cycle-start usb`

Next Wiring Tasks
1. Confirm sampled bit mapping against logic analyzer captures.
2. Verify sustained capture throughput for expected FRED transaction bursts.
3. Validate concurrent capture while master commands are active.

Notes
- `PIO1` is used as a passive sampler during bus-master operation; it does not drive the external bus.
