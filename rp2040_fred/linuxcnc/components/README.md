# TCL125 LinuxCNC Userspace Components

Two experimental HAL userspace components expose the same `tcl125.*` pins, so
switch between them by changing the `loadusr` line in `linuxcnc/tcl125.hal`.

- `tcl125_hal.py` follows LinuxCNC command position by comparing command to
  machine feedback. This is useful for debugging real following error, but it
  can hunt unless the deadband is wider than feedback quantization.
- `tcl125_delta_hal.py` sends motion from command-position deltas:
  `current motor-pos-cmd - last sent motor-pos-cmd`. Feedback is still reported
  to LinuxCNC, but small feedback noise does not generate correction moves.
- Both components wire the same spindle pins. `spindle-fwd` emits the inferred
  forward start block, `spindle-rev` emits the captured reverse start block,
  and `spindle-on` false emits the captured stop block.
- Both components request remote RPM service (`FCAD`) when enabling telemetry.

Example switch:

```hal
loadusr -W -n tcl125 ./components/tcl125_delta_hal.py
```

Useful environment overrides:

- `TCL125_JOG_DEADBAND_MM=0.05`
- `TCL125_SETTLE_TOLERANCE_MM=0.05`
- `TCL125_JOG_MAX_DELTA_MM=1.0`
- `TCL125_JOG_FEED=100`
- `TCL125_TRACE_PATH=/tmp/tcl125_delta_hal_trace.csv`
- `TCL125_SPINDLE_AT_SPEED_TOLERANCE_RPM=100`
