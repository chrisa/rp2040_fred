#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from experiment_common import (
    csv_output,
    drain_experiment,
    ensure_feedback_period,
    write_event,
)
from fred_client import FredUsbClient


def rounded_counts(delta_mm: float, counts_per_mm: float) -> int:
    return int(round(delta_mm * counts_per_mm))


def x_radius_counts_from_mm(delta_mm: float, counts_per_mm: float) -> int:
    x_diameter_counts = rounded_counts(delta_mm, counts_per_mm)
    if x_diameter_counts % 2:
        x_diameter_counts += 1 if x_diameter_counts > 0 else -1
    return x_diameter_counts // 2


def word(value: int) -> bytes:
    return int(value).to_bytes(2, "little", signed=True)


def command_payload(
    *,
    mode: str,
    x_mm: float,
    z_mm: float,
    feed: int,
    slew: int,
    x_counts_per_mm: float,
    z_counts_per_mm: float,
) -> bytes:
    x_radius = x_radius_counts_from_mm(x_mm, x_counts_per_mm)
    z_counts = rounded_counts(z_mm, z_counts_per_mm)
    m8 = 0 if mode == "rapid" else int(95_000 / feed)

    payload = bytearray(20)
    payload[0] = 0 if mode == "rapid" else 1
    payload[1] = 0
    payload[2:4] = word(x_radius)
    payload[4:6] = word(z_counts)
    payload[12:14] = int(m8).to_bytes(2, "little")
    payload[14:16] = int(slew).to_bytes(2, "little")
    return bytes(payload)


def write_block_ops(payload: bytes) -> list[dict[str, int | str]]:
    return [
        {"op": "write_gated", "addr": 0x92 + offset, "value": value}
        for offset, value in enumerate(payload)
    ]


def spindle_stop_payload() -> bytes:
    payload = bytearray(20)
    payload[0] = 0
    payload[1] = 5
    return bytes(payload)


def preset_script(
    name: str,
    *,
    delay_us: int,
    mode: str,
    x_mm: float,
    z_mm: float,
    feed: int,
    slew: int,
    x_counts_per_mm: float,
    z_counts_per_mm: float,
    custom_json: str | None,
) -> list[dict[str, int | str]]:
    if name == "custom-json":
        if custom_json is None:
            raise ValueError("--custom-json is required with --experiment custom-json")
        data = json.loads(Path(custom_json).read_text())
        if not isinstance(data, list):
            raise ValueError("custom JSON script must be a list of operation objects")
        return data

    script: list[dict[str, int | str]] = []
    if delay_us > 0:
        script.append({"op": "delay_us", "delay_us": delay_us})

    if name == "control":
        return script
    if name == "clear-af":
        script.append({"op": "write_gated", "addr": 0xAF, "value": 0})
        return script
    if name == "overwrite-zero":
        for addr in (0x94, 0x95, 0x96, 0x97):
            script.append({"op": "write_gated", "addr": addr, "value": 0})
        return script
    if name == "spindle-stop":
        script.extend(write_block_ops(spindle_stop_payload()))
        return script
    if name == "extend-same":
        payload = command_payload(
            mode=mode,
            x_mm=x_mm,
            z_mm=z_mm,
            feed=feed,
            slew=slew,
            x_counts_per_mm=x_counts_per_mm,
            z_counts_per_mm=z_counts_per_mm,
        )
        script.extend(write_block_ops(payload))
        return script

    raise ValueError(f"unknown experiment preset: {name}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Try timed in-flight bus interventions during an accepted TCL125 move."
    )
    parser.add_argument("--yes", action="store_true", help="required; physically moves axes")
    parser.add_argument("--vid", type=lambda value: int(value, 0), default=0x2E8A)
    parser.add_argument("--pid", type=lambda value: int(value, 0), default=0x000A)
    parser.add_argument("--x-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--z-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--axis", choices=("x", "z"), default="z")
    parser.add_argument("--direction", choices=("+", "-"), default="+")
    parser.add_argument("--length-mm", type=float, default=10.0)
    parser.add_argument("--mode", choices=("rapid", "feed"), default="feed")
    parser.add_argument("--feed", type=int, default=100)
    parser.add_argument("--slew", type=int, default=61)
    parser.add_argument("--feedback-period-ms", type=int, default=10)
    parser.add_argument("--allow-fast-feedback", action="store_true")
    parser.add_argument("--delay-us", type=int, default=100_000)
    parser.add_argument("--safe-mm", type=float, default=50.0)
    parser.add_argument(
        "--experiment",
        choices=("control", "clear-af", "overwrite-zero", "spindle-stop", "extend-same", "custom-json"),
        default="control",
    )
    parser.add_argument("--custom-json")
    parser.add_argument("--timeout-s", type=float, default=120.0)
    parser.add_argument("--out", default="-", help="CSV output path, or '-' for stdout")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    ensure_feedback_period(args.feedback_period_ms, args.allow_fast_feedback)
    if abs(args.length_mm) > args.safe_mm:
        raise ValueError("--length-mm exceeds --safe-mm")

    sign = 1.0 if args.direction == "+" else -1.0
    x_mm = sign * args.length_mm if args.axis == "x" else 0.0
    z_mm = sign * args.length_mm if args.axis == "z" else 0.0
    script = preset_script(
        args.experiment,
        delay_us=args.delay_us,
        mode=args.mode,
        x_mm=x_mm,
        z_mm=z_mm,
        feed=args.feed,
        slew=args.slew,
        x_counts_per_mm=args.x_counts_per_mm,
        z_counts_per_mm=args.z_counts_per_mm,
        custom_json=args.custom_json,
    )

    print("Planned in-flight experiment:", file=sys.stderr)
    print(
        f"  experiment={args.experiment} axis={args.axis}{args.direction} length={args.length_mm} mode={args.mode}",
        file=sys.stderr,
    )
    print(
        f"  delay_us={args.delay_us} feedback_period_ms={args.feedback_period_ms} script_ops={len(script)}",
        file=sys.stderr,
    )

    if not args.yes:
        print("Refusing to move without --yes.", file=sys.stderr)
        return 2

    output_fh, writer = csv_output(args.out)
    meta = {
        "trial_id": 1,
        "axis": args.axis,
        "direction": f"{args.axis}{args.direction}",
        "mode": args.mode,
        "length_mm": args.length_mm,
        "feed": args.feed if args.mode == "feed" else "",
        "slew": args.slew,
        "feedback_period_ms": args.feedback_period_ms,
        "script_name": args.experiment,
    }

    try:
        with FredUsbClient(
            args.vid,
            args.pid,
            timeout_ms=int(args.timeout_s * 1000),
            x_counts_per_mm=args.x_counts_per_mm,
            z_counts_per_mm=args.z_counts_per_mm,
        ) as client:
            client.enable_polling(
                period_ms=args.feedback_period_ms,
                rpm_service="remote",
            )
            try:
                client.refresh(timeout_ms=1000)
                write_event(writer, meta, "command_before")
                sent = client.run_experiment_move_delta(
                    x_mm=x_mm,
                    z_mm=z_mm,
                    mode=args.mode,
                    feed=args.feed,
                    slew=args.slew,
                    feedback_period_ms=args.feedback_period_ms,
                    trial_id=1,
                    script_ops=script,
                )
                write_event(writer, meta, "command_after_sent" if sent else "skipped")
                if sent:
                    drain_experiment(
                        client,
                        writer,
                        meta,
                        x_counts_per_mm=args.x_counts_per_mm,
                        z_counts_per_mm=args.z_counts_per_mm,
                        timeout_s=args.timeout_s,
                    )
            finally:
                client.disable_polling()
        return 0
    finally:
        if output_fh is not sys.stdout:
            output_fh.close()


if __name__ == "__main__":
    raise SystemExit(main())
