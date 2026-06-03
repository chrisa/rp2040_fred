#!/usr/bin/env python3

from __future__ import annotations

import argparse
import itertools
import sys
import time
from typing import Iterable, Tuple

from experiment_common import (
    assert_safe_sequence,
    csv_output,
    drain_experiment,
    ensure_feedback_period,
    parse_csv_floats,
    parse_csv_ints,
    write_event,
)
from fred_client import FredUsbClient


def axis_moves(axis: str, length_mm: float) -> Iterable[Tuple[str, float, float]]:
    if axis == "x":
        yield ("x+", length_mm, 0.0)
        yield ("x-", -length_mm, 0.0)
    elif axis == "z":
        yield ("z+", 0.0, length_mm)
        yield ("z-", 0.0, -length_mm)
    else:
        raise ValueError(f"unknown axis: {axis}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Capture raw TCL125 motion latency samples from Python-driven moves."
    )
    parser.add_argument("--yes", action="store_true", help="required; physically moves axes")
    parser.add_argument("--vid", type=lambda value: int(value, 0), default=0x2E8A)
    parser.add_argument("--pid", type=lambda value: int(value, 0), default=0x000A)
    parser.add_argument("--x-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--z-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--axes", choices=("x", "z", "both"), default="both")
    parser.add_argument("--length-mm", default="1,5,10")
    parser.add_argument("--mode", choices=("rapid", "feed", "both"), default="both")
    parser.add_argument("--feed", default="100")
    parser.add_argument("--slew", default="61")
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--feedback-period-ms", type=int, default=10)
    parser.add_argument("--allow-fast-feedback", action="store_true")
    parser.add_argument("--safe-mm", type=float, default=50.0)
    parser.add_argument("--timeout-s", type=float, default=120.0)
    parser.add_argument("--settle-s", type=float, default=0.05)
    parser.add_argument("--out", default="-", help="CSV output path, or '-' for stdout")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    ensure_feedback_period(args.feedback_period_ms, args.allow_fast_feedback)

    axes = ["x", "z"] if args.axes == "both" else [args.axes]
    modes = ["rapid", "feed"] if args.mode == "both" else [args.mode]
    lengths = parse_csv_floats(args.length_mm)
    feeds = parse_csv_ints(args.feed)
    slews = parse_csv_ints(args.slew)

    planned_moves: list[Tuple[str, float, float]] = []
    for axis, length_mm in itertools.product(axes, lengths):
        for _label, x_mm, z_mm in axis_moves(axis, length_mm):
            planned_moves.append((axis, x_mm, z_mm))
    assert_safe_sequence(planned_moves, args.safe_mm)

    print("Planned sweep:", file=sys.stderr)
    print(f"  axes={axes} lengths={lengths} modes={modes}", file=sys.stderr)
    print(f"  feeds={feeds} slews={slews} trials={args.trials}", file=sys.stderr)
    print(f"  feedback_period_ms={args.feedback_period_ms}", file=sys.stderr)

    if not args.yes:
        print("Refusing to move without --yes.", file=sys.stderr)
        return 2

    output_fh, writer = csv_output(args.out)
    trial_id = 1

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
                for mode in modes:
                    feed_values = feeds if mode == "feed" else [0]
                    for axis, length_mm, feed, slew in itertools.product(
                        axes, lengths, feed_values, slews
                    ):
                        for trial in range(args.trials):
                            for direction, x_mm, z_mm in axis_moves(axis, length_mm):
                                meta = {
                                    "trial_id": trial_id,
                                    "axis": axis,
                                    "direction": direction,
                                    "mode": mode,
                                    "length_mm": length_mm,
                                    "feed": feed if mode == "feed" else "",
                                    "slew": slew,
                                    "feedback_period_ms": args.feedback_period_ms,
                                    "script_name": "latency",
                                }
                                write_event(writer, meta, "command_before")
                                sent = client.run_experiment_move_delta(
                                    x_mm=x_mm,
                                    z_mm=z_mm,
                                    mode=mode,
                                    feed=feed if mode == "feed" else 100,
                                    slew=slew,
                                    feedback_period_ms=args.feedback_period_ms,
                                    trial_id=trial_id,
                                    script_ops=[],
                                )
                                write_event(
                                    writer,
                                    meta,
                                    "command_after_sent" if sent else "skipped",
                                )
                                if sent:
                                    drain_experiment(
                                        client,
                                        writer,
                                        meta,
                                        x_counts_per_mm=args.x_counts_per_mm,
                                        z_counts_per_mm=args.z_counts_per_mm,
                                        timeout_s=args.timeout_s,
                                    )
                                if args.settle_s > 0:
                                    time.sleep(args.settle_s)
                                trial_id += 1
            finally:
                client.disable_polling()
        return 0
    finally:
        if output_fh is not sys.stdout:
            output_fh.close()


if __name__ == "__main__":
    raise SystemExit(main())
