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
- `cargo run --offline -- move usb --mode rapid --x-counts 0 --z-counts 100 --slew 61`
- `cargo run --offline -- move usb --mode feed --x-counts 0 --z-counts 100 --feed 100 --slew 61`
- `cargo run --offline -- move usb --mode rapid --x-counts 0 --z-counts 100 --slew 61 --suspend-polling --wait-complete`
- `cargo run --offline -- move usb --mode rapid --x-counts 0 --z-counts 100 --slew 61 --arm-wait --arm-x-counts 0 --arm-z-counts 0 --wait-complete`
- `cargo run --offline -- tool usb --current-station 1 --target-station 2 --slew 61 --wait-complete`
- `cargo run --offline -- arm-wait usb --x-counts 0 --z-counts 0 --suspend-polling`
- `cargo run --offline -- capture-on usb`
- `cargo run --offline -- capture-off usb`
- `cargo run --offline -- capture usb`

Notes
- X display uses diameter semantics (`x_counts * 2`) to match CNCMAN behavior.
- Z display uses direct axis counts.
- Mock telemetry emits one packet per full 10-command DRO cadence.
- Default USB target is `VID=0x2E8A`, `PID=0x000A`, with the first bulk IN/OUT interface discovered at runtime.
- Firmware capture mode is supported alongside bus-master mode; `monitor usb` automatically disables capture and enables DRO telemetry.
- `move usb` sends one low-level `CommandBlock`; X deltas are entered as diameter counts and must be even because the controller payload uses radius counts.
- Command-block execution follows the observed old-host sequence: load `FC92..FCA5`, run a `FC80=00` busy/ready cycle, then clear with `FCAF=00`.
- `--suspend-polling` asks the firmware to stop normal position/RPM polling while the queued controller work is pending or active.
- `--wait-complete` makes the host poll controller status and return only after the queued work and controller busy-wait are complete.
- `arm-wait usb --suspend-polling` asks the firmware to suppress normal position/RPM polling around the pre-start blocks and `PROCcont` wait.
- `--arm-wait` queues the CNCMAK1 pre-start shape before the move: X-only `do(0,0,arm_x,0,0,0,0,0,0,0)`, Z-only `do(0,0,0,arm_z,0,0,0,0,0,0)`, then the `PROCcont`-style cycle-start wait.
- Standalone `arm-wait usb` queues the same two pre-start blocks and returns after `PROCcont` completes.
- Firmware bounds `PROCcont` waits and command-block execute waits; timeout is reported through controller status so the host returns an error instead of leaving feedback permanently wedged.
- The old manufacturing program sends those two `do(0,0,...)` blocks before displaying `PRESS START CYCLE`; they may be more than simple visible positioning from the controller's point of view.
- `tool usb` implements the CNCMAK1 automatic turret path for `M06 ... K=<station>`.
  It needs the current station because the old host tracks `C%` locally and emits relative turret motion.
- Tool number `I` and turret station `K` are separate in the old host; `tool usb` moves/selects the physical turret station, not a host-side tool offset record.
- Conversion constants currently default to:
  - `x_counts_per_mm = 100`
  - `z_counts_per_mm = 100`
  and should be calibrated against real machine movement.
