#!/usr/bin/env python3

from __future__ import annotations

import argparse
import sys
import time

from experiment_common import (
    csv_output,
    drain_experiment,
    ensure_feedback_period,
    write_event,
)
from fred_client import FredUsbClient


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Capture firmware-side timing for TCL125 feedback polling without axis motion."
    )
    parser.add_argument("--vid", type=lambda value: int(value, 0), default=0x2E8A)
    parser.add_argument("--pid", type=lambda value: int(value, 0), default=0x000A)
    parser.add_argument("--x-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--z-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--feedback-period-ms", type=int, default=10)
    parser.add_argument("--allow-fast-feedback", action="store_true")
    parser.add_argument("--sequence", choices=("full", "xz", "x", "z"), default="full")
    parser.add_argument("--poll-count", type=int, default=30)
    parser.add_argument("--trials", type=int, default=1)
    parser.add_argument("--settle-s", type=float, default=0.05)
    parser.add_argument("--timeout-s", type=float, default=60.0)
    parser.add_argument("--out", default="-", help="CSV output path, or '-' for stdout")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    ensure_feedback_period(args.feedback_period_ms, args.allow_fast_feedback)
    if args.poll_count <= 0:
        raise ValueError("--poll-count must be greater than zero")
    if args.trials <= 0:
        raise ValueError("--trials must be greater than zero")

    print("Planned feedback timing probe:", file=sys.stderr)
    print(
        f"  sequence={args.sequence} poll_count={args.poll_count} trials={args.trials}",
        file=sys.stderr,
    )
    print(f"  feedback_period_ms={args.feedback_period_ms}", file=sys.stderr)

    output_fh, writer = csv_output(args.out)
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
                for trial_id in range(1, args.trials + 1):
                    meta = {
                        "trial_id": trial_id,
                        "axis": "",
                        "direction": "",
                        "mode": "feedback",
                        "length_mm": "",
                        "feed": "",
                        "slew": "",
                        "feedback_period_ms": args.feedback_period_ms,
                        "script_name": f"feedback-timing-{args.sequence}",
                    }
                    write_event(writer, meta, "feedback_before")
                    client.run_feedback_timing_experiment(
                        feedback_period_ms=args.feedback_period_ms,
                        trial_id=trial_id,
                        poll_count=args.poll_count,
                        sequence=args.sequence,
                    )
                    write_event(writer, meta, "feedback_after_sent")
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
            finally:
                client.disable_polling()
        return 0
    finally:
        if output_fh is not sys.stdout:
            output_fh.close()


if __name__ == "__main__":
    raise SystemExit(main())
