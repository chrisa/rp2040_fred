# TCL125 Hall Homing Plan

## Summary

The machine has 100 mm of travel per axis and accepts autonomous movement
commands that cannot be interrupted after they start. Homing therefore uses
bounded moves and observations made during those moves. The controller is never
asked to stop on a switch edge. Instead, host-side code records Hall transitions
and controller telemetry while each bounded move is in progress, then decides
what to do after the move completes.

The first implementation is host-side only:

- the real machine backend will issue movement through the existing host/Pico
  USB transport and read Hall inputs from host GPIO;
- the initial implementation adds a simulated backend and visualization so the
  homing state machine can be tested before wiring sensors.

## Sensor Layout

Coordinate convention per axis:

- `0.0 mm`: chuck/unsafe hard stop;
- `100.0 mm`: safe/home hard stop;
- homing direction: positive, toward `100.0 mm`.

Each axis uses two Hall inputs:

| Sensor | Active zone | Purpose |
| --- | --- | --- |
| `coarse` | `85.0 mm <= pos <= 100.0 mm` | End the long rapid-search phase before the final home region. |
| `home` | `98.0 mm <= pos <= 100.0 mm` | Repeatable latch edge and machine datum. |

The meaningful event is the rising edge of each active zone as the axis moves
positive. The active zone should continue to the safe hard-stop end so the
machine can tell when it starts beyond the edge.

Physical target guidance:

- `coarse`: use a magnetic strip or target with a clean leading edge at
  `85.0 mm` and an active zone of roughly 15 mm inside travel. A physical
  magnet/strip of about `20-25 mm` gives room for field fringe and mounting
  tolerance.
- `home`: use a separate target with a clean leading edge at `98.0 mm`. The
  active zone inside travel is only 2 mm, so use an `8-12 mm` physical target if
  it can extend beyond the nominal travel region. If not, use the longest target
  that fits and characterize the real edge.
- Mount the tracks with enough lateral separation that each Hall only sees its
  own target.
- Sensor polarity is software-configurable.

The home datum is the measured `home` rising edge. With the edge at `98.0 mm`
and the hard stop at `100.0 mm`, the repeatable home edge has 2 mm hard-stop
clearance. The initial software limits should be `1.0 mm .. 99.0 mm`, leaving
about 1 mm software clearance at either end.

## Homing Sequence

Home axes sequentially: Z first, then X.

For each axis:

1. Read the current Hall state and controller position feedback.
2. If `home` is active, move negative in `1.0 mm` blocks at `250 mm/min` until
   `home` releases. Then move another `1.0 mm` negative for clearance. Fault if
   `home` does not release within `8.0 mm`.
3. If neither sensor is active, fast-search positive in `10.0 mm` rapid blocks
   at about `500 mm/min`. Stop the fast phase as soon as `coarse` becomes
   active. Fault if `home` is seen before `coarse`, if total search exceeds
   `110.0 mm`, or if a hard stop is reached.
4. Near-home search positive in `1.0 mm` feed blocks at `250 mm/min` until
   `home` becomes active. Fault if `home` is not seen within `18.0 mm` after
   the coarse region, or if a hard stop is reached.
5. Back off negative by `1.0 mm` at `250 mm/min` and confirm `home` is inactive.
6. Slow-latch positive in `0.25 mm` blocks at `25 mm/min`. Record the first
   `home` rising edge.
7. Repeat the slow latch 3 times, backing off `1.0 mm` between latch attempts.
   Use the median edge position as the datum.
8. Apply a coordinate offset so the median `home` edge is machine position
   `98.000 mm`.
9. Move to post-home park at `96.0 mm`.

## Failure Rules

Homing must not declare success if:

- `home` does not release during the initial backoff;
- `coarse` is not seen before the final home region;
- `home` is detected during the fast-search phase;
- `home` does not appear during the near-home or slow-latch phases;
- `home` remains active after latch backoff;
- a hard stop is reached in a normal homing run;
- feedback does not plausibly track commanded motion;
- simulation detects missing, stuck, inverted, or noisy sensors outside the
  configured tolerance.

## Simulation Harness

The simulator should exercise the same host-side homing state machine that the
real machine backend will use later.

The backend abstraction should provide:

- current position/telemetry for an axis;
- current `coarse` and `home` Hall states;
- non-interruptible axis moves;
- move observations collected while the move was active;
- coordinate offset application after homing.

The simulated machine should model:

- axis position from `0.0 mm` to `100.0 mm`;
- hard-stop contact;
- non-interruptible moves at configured feed/rapid rates;
- Hall zones with polarity, missing/stuck behavior, and edge jitter;
- controller telemetry at a configurable period, default `10 ms`;
- deterministic default behavior;
- optional telemetry imperfections: dropped snapshots, delayed snapshots,
  stale-last-snapshot behavior, quantized feedback, and noisy feedback;
- optional Hall imperfections: edge jitter, polarity inversion, and stuck or
  missing states.

Default simulation starts:

- `0.0, 25.0, 50.0, 84.0, 86.0, 90.0, 97.0, 98.5, 99.5 mm`.

Acceptance:

- normal starts home without hard-stop contact;
- starts inside the home zone back off and re-latch;
- missing/inverted sensors fault;
- dropped telemetry snapshots do not break homing unless they exceed configured
  tolerance;
- noisy or quantized position feedback still produces a stable median latch;
- the simulation reports total time to home and move count for every scenario.

## Visualization

The simulator should be able to write an HTML report that opens directly in a
browser. The report should show:

- position versus time;
- commanded motion blocks;
- `coarse` and `home` Hall digital traces;
- controller telemetry samples, including dropped or stale samples;
- detected Hall edges;
- latch attempts and the median home edge;
- hard-stop contact, if any;
- final result summary: success/fault, total time, move count, computed home
  datum.

For `--all`, prefer one combined report with a panel per scenario.
