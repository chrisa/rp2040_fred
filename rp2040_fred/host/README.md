fredctl (Host Bring-Up Tool)
============================

Status
- Shared bridge packet protocol in `rp2040-fred-protocol`.
- Firmware-side bridge service with request handling, telemetry event generation, and queued controller work.
- Host library and CLI for live USB monitor, capture, cycle-start, motion,
  tool-change, spindle, G33 thread-sync, and canned-cycle tests.
- `usb` transport is implemented with `rusb` against role-specific vendor bulk interfaces.
- Python/PyO3 consumes the host library so LinuxCNC can use one master USB connection for feedback, status, and jog commands.

Usage (usb mode)
- `cargo run --offline -- monitor usb`
- `cargo run --offline -- cycle-move usb --mode rapid --x-counts 0 --z-counts 100 --slew 61`
- `cargo run --offline -- cycle-move usb --mode feed --x-counts 0 --z-counts 100 --feed 100 --slew 61`
- `cargo run --offline -- jog usb --mode rapid --x-counts 0 --z-counts 100 --slew 61`
- `cargo run --offline -- jog usb --mode feed --x-counts 0 --z-counts 100 --feed 100 --slew 61`
- `cargo run --offline -- tool usb --current-station 1 --target-station 2 --slew 61 --wait-complete`
- `cargo run --offline -- spindle usb --start reverse --ssl 125 --wait-complete`
- `cargo run --offline -- spindle usb --start forward --rpm 3000 --wait-complete`
- `cargo run --offline -- spindle usb --stop --wait-complete`
- `cargo run --offline -- cycle-start usb`
- `cargo run --offline -- g33 usb --z-mm -15 --pitch-mm 1.5 --slew 61 --wait-complete`
- `cargo run --offline -- canned-cycle usb --code G80`
- `cargo run --offline -- capture usb`

Notes
- X display uses diameter semantics (`x_counts * 2`) to match CNCMAN behavior.
- Z display uses direct axis counts.
- Default USB target is `VID=0x2E8A`, `PID=0x000A`.
- Firmware exposes two vendor-specific USB interfaces: master protocol `0x01` for monitor/motion/controller commands, and capture protocol `0x02` for passive trace streaming.
- Firmware still has a startup-selected passive transport; in that mode `capture usb` uses the capture interface, while motion/controller commands are not available.
- Because the host claims the master and capture interfaces separately, `capture usb` can run in one process while `monitor usb`, `cycle-move usb`, `jog usb`, `cycle-start usb`, `tool usb`, or `spindle usb` runs in another.
- `cycle-move usb` waits for the observed cycle-start/continue condition (`FCF0 & 0x10 == 0`) before sending one low-level `CommandBlock`.
- `monitor usb` requests manual-mode RPM service (`FC88`) so it can be used to
  prove the manual feedback path.
- `jog usb` sends the same low-level `CommandBlock` immediately for remote-jog testing.
- X deltas are entered as diameter counts and must be even because the controller payload uses radius counts.
- Controller work always pauses normal position/RPM polling while queued or active so background feedback does not compete with motion-command writes.
- Command-block staging follows the observed old-host sequence: wait for `FCF0` bit 0 clear before each `FC92..FCA5` byte, read the target register, write it, run a PROCbusy-style feedback service loop until `FCF0` bit 7 clears, then clear with gated `FCAF=00`.
- Motion commands make the host poll controller status and return only after the queued work and controller busy-wait are complete.
- Standalone `cycle-start usb` tests the Rust-side cycle-start listener without sending motion.
- Firmware bounds cycle-start waits and command-block busy waits; timeout is reported through controller status so the host returns an error instead of leaving feedback permanently wedged.
- `tool usb` implements the CNCMAK1 automatic turret path for `M06 ... K=<station>`.
  It needs the current station because the old host tracks `C%` locally and emits relative turret motion.
- Tool number `I` and turret station `K` are separate in the old host; `tool usb` moves/selects the physical turret station, not a host-side tool offset record.
- `spindle usb` implements the captured `do(0,3/4/5,...)` spindle family:
  `m2=3` starts reverse, `m2=4` starts forward, and `m2=5` stops.  `--ssl`
  sends the raw old-host speed code; `--rpm` converts approximately as
  `SSL = rpm / 24` and caps at 127.
- `g33 usb` sends one decoded spindle-synchronized thread pass using opcode
  `84`.  Full `G84` threading cycle expansion belongs in the caller so previews
  and pass scheduling can use the same plan that is executed.
- `canned-cycle usb` currently accepts `G80` as cancel/no-op and validates
  `G81` through `G83` parameters, but refuses to move until those command-block
  mappings are decoded.
- Conversion constants currently default to:
  - `x_counts_per_mm = 100`
  - `z_counts_per_mm = 100`
  and should be calibrated against real machine movement.
- LinuxCNC jog-first integration uses immediate rapid deltas from the Python layer.
  The userspace HAL component coalesces changing `motor-pos-cmd` targets and only keeps one controller command active at a time.
- Spindle command pins are wired through LinuxCNC and send start/stop blocks
  through the same queued controller command path as motion and tool change.
- Python/LinuxCNC polling requests remote-mode RPM service (`FCAD`) because
  manual-mode `FC88` RPM triggering returns zeros while the bridge owns the
  machine.
