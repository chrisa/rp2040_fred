#!/usr/bin/python3

from __future__ import annotations

import csv
import os
import sys
import time
from typing import Optional, Tuple

import hal

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

from fred_client import FredProtocolError, FredUsbClient, FredUsbError  # noqa: E402


def env_float(name: str, default: float) -> float:
    return float(os.environ.get(name, str(default)))


def env_int(name: str, default: int) -> int:
    return int(os.environ.get(name, str(default)), 0)


def env_str(name: str, default: str) -> str:
    return os.environ.get(name, default)


CYCLE_HZ = env_float("TCL125_HAL_HZ", 100.0)
CYCLE_S = 1.0 / CYCLE_HZ
USB_VID = env_int("TCL125_USB_VID", 0x2E8A)
USB_PID = env_int("TCL125_USB_PID", 0x000A)
USB_TIMEOUT_MS = env_int("TCL125_USB_TIMEOUT_MS", 1000)
USB_RECONNECT_S = env_float("TCL125_USB_RECONNECT_S", 1.0)
POLL_PERIOD_MS = env_int("TCL125_POLL_PERIOD_MS", 10)
X_COUNTS_PER_MM = env_float("TCL125_X_COUNTS_PER_MM", 100.0)
Z_COUNTS_PER_MM = env_float("TCL125_Z_COUNTS_PER_MM", 100.0)
JOG_SLEW = env_int("TCL125_JOG_SLEW", 61)
JOG_FEED = env_int("TCL125_JOG_FEED", 100)
JOG_DEADBAND_MM = env_float("TCL125_JOG_DEADBAND_MM", 0.05)
JOG_MAX_DELTA_MM = env_float("TCL125_JOG_MAX_DELTA_MM", 1.0)
SETTLE_TOLERANCE_MM = env_float("TCL125_SETTLE_TOLERANCE_MM", 0.05)
SETTLE_TIMEOUT_S = env_float("TCL125_SETTLE_TIMEOUT_S", 10.0)
TRACE_PATH = env_str("TCL125_TRACE_PATH", "/tmp/tcl125_delta_hal_trace.csv")
SPINDLE_AT_SPEED_TOLERANCE_RPM = env_float("TCL125_SPINDLE_AT_SPEED_TOLERANCE_RPM", 100.0)
SPINDLE_MAX_COMMANDED_RPM = env_float("TCL125_SPINDLE_MAX_COMMANDED_RPM", 127.0 * 24.0)


def clamp(value: float, limit: float) -> float:
    return max(-limit, min(limit, value))


def limited_delta(
    current_target: Tuple[float, float],
    sent_target: Tuple[float, float],
) -> Tuple[float, float]:
    dx = current_target[0] - sent_target[0]
    dz = current_target[1] - sent_target[1]

    if abs(dx) < JOG_DEADBAND_MM:
        dx = 0.0
    if abs(dz) < JOG_DEADBAND_MM:
        dz = 0.0

    return (clamp(dx, JOG_MAX_DELTA_MM), clamp(dz, JOG_MAX_DELTA_MM))


def has_motion(delta: Tuple[float, float]) -> bool:
    return delta[0] != 0.0 or delta[1] != 0.0


def within_tolerance(
    actual: Tuple[float, float],
    expected: Tuple[float, float],
    tolerance: float,
) -> bool:
    return abs(actual[0] - expected[0]) <= tolerance and abs(actual[1] - expected[1]) <= tolerance


def current_spindle_state() -> Tuple[bool, bool, bool, float]:
    on = bool(h["spindle-on"])
    if not on:
        return (False, False, False, 0.0)
    return (
        True,
        bool(h["spindle-fwd"]),
        bool(h["spindle-rev"]),
        abs(float(h["spindle-speed-cmd"])),
    )


def spindle_forward(fwd: bool, rev: bool) -> bool:
    if rev:
        return False
    if fwd:
        return True
    return True


def spindle_at_speed(on: bool, commanded_rpm: float, actual_rpm: float) -> bool:
    if not on:
        return True
    target = min(abs(commanded_rpm), SPINDLE_MAX_COMMANDED_RPM)
    if target <= SPINDLE_AT_SPEED_TOLERANCE_RPM:
        return True
    return abs(abs(actual_rpm) - target) <= SPINDLE_AT_SPEED_TOLERANCE_RPM


h = hal.component("tcl125")

h.newpin("x-pos-cmd", hal.HAL_FLOAT, hal.HAL_IN)
h.newpin("z-pos-cmd", hal.HAL_FLOAT, hal.HAL_IN)
h.newpin("x-pos-fb", hal.HAL_FLOAT, hal.HAL_OUT)
h.newpin("z-pos-fb", hal.HAL_FLOAT, hal.HAL_OUT)

h.newpin("spindle-speed-cmd", hal.HAL_FLOAT, hal.HAL_IN)
h.newpin("spindle-on", hal.HAL_BIT, hal.HAL_IN)
h.newpin("spindle-fwd", hal.HAL_BIT, hal.HAL_IN)
h.newpin("spindle-rev", hal.HAL_BIT, hal.HAL_IN)
h.newpin("spindle-rpm", hal.HAL_FLOAT, hal.HAL_OUT)
h.newpin("spindle-at-speed", hal.HAL_BIT, hal.HAL_OUT)

h.newpin("usb-connected", hal.HAL_BIT, hal.HAL_OUT)
h.newpin("controller-active", hal.HAL_BIT, hal.HAL_OUT)
h.newpin("controller-error", hal.HAL_BIT, hal.HAL_OUT)

h.ready()


trace_file = None
trace_writer = None
if TRACE_PATH:
    try:
        trace_exists = os.path.exists(TRACE_PATH) and os.path.getsize(TRACE_PATH) > 0
        trace_file = open(TRACE_PATH, "a", newline="", buffering=1)
        trace_writer = csv.writer(trace_file)
        if not trace_exists:
            trace_writer.writerow(
                [
                    "time_s",
                    "x_cmd",
                    "z_cmd",
                    "x_sent",
                    "z_sent",
                    "x_fb",
                    "z_fb",
                    "dx",
                    "dz",
                    "expected_x",
                    "expected_z",
                    "snapshot_tick",
                    "generation",
                    "command_active",
                    "status_idle",
                    "settling",
                ]
            )
        print(f"tcl125-delta: writing command trace to {TRACE_PATH}")
    except OSError as exc:
        trace_file = None
        trace_writer = None
        print(f"tcl125-delta: command trace disabled: {exc}")


def trace_command(
    now_s: float,
    target: Tuple[float, float],
    sent_target: Tuple[float, float],
    feedback: Optional[Tuple[float, float]],
    delta: Tuple[float, float],
    expected: Tuple[float, float],
    snapshot_tick: Optional[int],
    generation: int,
    command_active: bool,
    status_idle: Optional[bool],
    settling: bool,
) -> None:
    if trace_writer is None:
        return

    x_fb = "" if feedback is None else f"{feedback[0]:.6f}"
    z_fb = "" if feedback is None else f"{feedback[1]:.6f}"
    trace_writer.writerow(
        [
            f"{now_s:.6f}",
            f"{target[0]:.6f}",
            f"{target[1]:.6f}",
            f"{sent_target[0]:.6f}",
            f"{sent_target[1]:.6f}",
            x_fb,
            z_fb,
            f"{delta[0]:.6f}",
            f"{delta[1]:.6f}",
            f"{expected[0]:.6f}",
            f"{expected[1]:.6f}",
            "" if snapshot_tick is None else snapshot_tick,
            generation,
            int(command_active),
            "" if status_idle is None else int(status_idle),
            int(settling),
        ]
    )


client: Optional[FredUsbClient] = None
next_connect_time = 0.0
feedback: Optional[Tuple[float, float]] = None
feedback_generation = 0
snapshot_tick: Optional[int] = None
last_sent_target: Optional[Tuple[float, float]] = None
command_active = False
settling_after_command = False
settle_target: Optional[Tuple[float, float]] = None
settle_deadline = 0.0
last_status_idle: Optional[bool] = None
last_spindle = (None, None, None, None)


def apply_snapshot(snapshot: dict[str, object]) -> None:
    global feedback, feedback_generation, snapshot_tick

    feedback = (float(snapshot["x_mm"]), float(snapshot["z_mm"]))
    feedback_generation = int(snapshot.get("generation", 0))
    snapshot_tick = int(snapshot.get("tick", 0))
    h["x-pos-fb"] = feedback[0]
    h["z-pos-fb"] = feedback[1]
    h["spindle-rpm"] = float(snapshot["spindle_rpm"])


def current_target() -> Tuple[float, float]:
    return (float(h["x-pos-cmd"]), float(h["z-pos-cmd"]))


def connect_if_due(now: float) -> Optional[FredUsbClient]:
    global next_connect_time

    if now < next_connect_time:
        return None

    try:
        new_client = FredUsbClient(
            USB_VID,
            USB_PID,
            timeout_ms=USB_TIMEOUT_MS,
            x_counts_per_mm=X_COUNTS_PER_MM,
            z_counts_per_mm=Z_COUNTS_PER_MM,
        )
        new_client.enable_polling(period_ms=POLL_PERIOD_MS)
        h["usb-connected"] = True
        h["controller-error"] = False
        print("tcl125-delta: connected to RP2040 FRED bridge")
        return new_client
    except (FredUsbError, FredProtocolError, OSError) as exc:
        h["usb-connected"] = False
        h["controller-active"] = False
        h["controller-error"] = True
        next_connect_time = now + USB_RECONNECT_S
        print(f"tcl125-delta: USB connect failed: {exc}")
        return None


def disconnect(reason: object) -> None:
    global client, command_active, settling_after_command, settle_target, next_connect_time, last_spindle

    print(f"tcl125-delta: USB disconnected: {reason}")
    if client is not None:
        try:
            client.close()
        except Exception:
            pass
    client = None
    command_active = False
    settling_after_command = False
    settle_target = None
    last_spindle = (None, None, None, None)
    next_connect_time = time.monotonic() + USB_RECONNECT_S
    h["usb-connected"] = False
    h["controller-active"] = False
    h["controller-error"] = True


try:
    while True:
        loop_start = time.monotonic()

        if client is None:
            client = connect_if_due(loop_start)

        if client is not None:
            try:
                controller_error = False
                snapshot = client.refresh(timeout_ms=0)
                if snapshot is None:
                    snapshot = client.latest_snapshot()

                if snapshot is not None:
                    apply_snapshot(snapshot)

                if last_sent_target is None:
                    last_sent_target = current_target()

                if settling_after_command and feedback is not None and settle_target is not None:
                    if within_tolerance(feedback, settle_target, SETTLE_TOLERANCE_MM):
                        settling_after_command = False
                        settle_target = None
                    elif loop_start >= settle_deadline:
                        print(
                            "tcl125-delta: settle timeout: "
                            f"fb=({feedback[0]:+.6f},{feedback[1]:+.6f}) "
                            f"expected=({settle_target[0]:+.6f},{settle_target[1]:+.6f})"
                        )
                        settling_after_command = False
                        settle_target = None

                if command_active:
                    status = client.controller_status()
                    last_status_idle = bool(status["idle"])
                    command_active = not last_status_idle
                    h["controller-active"] = command_active
                    controller_error = bool(status["error"])
                    h["controller-error"] = controller_error
                    if controller_error:
                        command_active = False
                        settling_after_command = False
                        settle_target = None
                    elif last_status_idle:
                        latest = client.latest_snapshot()
                        if latest is not None:
                            apply_snapshot(latest)
                        settling_after_command = settle_target is not None
                        settle_deadline = loop_start + SETTLE_TIMEOUT_S

                target = current_target()
                if (
                    not command_active
                    and not controller_error
                    and not settling_after_command
                    and last_sent_target is not None
                ):
                    delta = limited_delta(target, last_sent_target)
                    if has_motion(delta):
                        expected = (
                            last_sent_target[0] + delta[0],
                            last_sent_target[1] + delta[1],
                        )
                        trace_command(
                            loop_start,
                            target,
                            last_sent_target,
                            feedback,
                            delta,
                            expected,
                            snapshot_tick,
                            feedback_generation,
                            command_active,
                            last_status_idle,
                            settling_after_command,
                        )
                        command_active = client.feed_move_delta(
                            x_mm=delta[0],
                            z_mm=delta[1],
                            feed=JOG_FEED,
                            slew=JOG_SLEW,
                            wait=False,
                        )
                        if command_active:
                            last_sent_target = expected
                            settle_target = expected
                        h["controller-active"] = command_active

                spindle_state = current_spindle_state()
                if not command_active and not controller_error and spindle_state != last_spindle:
                    on, fwd, rev, rpm = spindle_state
                    forward = spindle_forward(fwd, rev)
                    command_active = client.set_spindle(
                        on=on,
                        rpm=rpm,
                        forward=forward,
                        wait=False,
                    )
                    last_spindle = spindle_state
                    h["controller-active"] = command_active
                    if on:
                        direction = "forward" if forward else "reverse"
                        print(
                            "tcl125-delta: spindle start queued: "
                            f"direction={direction} rpm={rpm:.1f}"
                        )
                    else:
                        print("tcl125-delta: spindle stop queued")

            except (FredUsbError, FredProtocolError, OSError) as exc:
                disconnect(exc)

        spindle_state = current_spindle_state()
        h["spindle-at-speed"] = spindle_at_speed(
            spindle_state[0],
            spindle_state[3],
            float(h["spindle-rpm"]),
        )

        elapsed = time.monotonic() - loop_start
        time.sleep(max(0.0, CYCLE_S - elapsed))

except KeyboardInterrupt:
    pass
finally:
    if client is not None:
        try:
            client.disable_polling()
        finally:
            client.close()
    if trace_file is not None:
        trace_file.close()
