#!/usr/bin/env python3

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the full TCL125 latency and in-flight intervention experiment series."
    )
    parser.add_argument("--yes", action="store_true", help="required; physically moves axes")
    parser.add_argument("--vid", type=lambda value: int(value, 0), default=0x2E8A)
    parser.add_argument("--pid", type=lambda value: int(value, 0), default=0x000A)
    parser.add_argument("--x-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--z-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--out-dir", default=None)
    parser.add_argument("--feedback-period-ms", type=int, default=10)
    parser.add_argument("--allow-fast-feedback", action="store_true")
    parser.add_argument("--skip-feedback-timing-probe", action="store_true")
    parser.add_argument("--feedback-timing-sequences", default="full,xz")
    parser.add_argument("--feedback-timing-poll-count", type=int, default=30)
    parser.add_argument("--feedback-timing-trials", type=int, default=1)
    parser.add_argument(
        "--motion-feedback-timing",
        action="store_true",
        help="include per-command timing rows in latency experiments",
    )
    parser.add_argument(
        "--intervention-feedback-timing",
        action="store_true",
        help="include per-command timing rows in in-flight intervention experiments",
    )
    parser.add_argument("--slew", type=int, default=61)
    parser.add_argument("--safe-mm", type=float, default=50.0)
    parser.add_argument("--latency-length-mm", default="1,5,10")
    parser.add_argument("--latency-trials", type=int, default=5)
    parser.add_argument("--latency-feed", type=int, default=100)
    parser.add_argument("--latency-timeout-s", type=float, default=180.0)
    parser.add_argument("--intervention-axis", choices=("x", "z"), default="z")
    parser.add_argument("--intervention-length-mm", type=float, default=10.0)
    parser.add_argument("--intervention-feed", type=int, default=20)
    parser.add_argument("--intervention-delay-us", type=int, default=20_000)
    parser.add_argument("--intervention-timeout-s", type=float, default=240.0)
    parser.add_argument(
        "--interventions",
        default="control,clear-af,overwrite-zero,spindle-stop,extend-same",
        help="comma-separated intervention presets",
    )
    return parser.parse_args()


def common_device_args(args: argparse.Namespace) -> list[str]:
    return [
        "--vid",
        hex(args.vid),
        "--pid",
        hex(args.pid),
        "--x-counts-per-mm",
        str(args.x_counts_per_mm),
        "--z-counts-per-mm",
        str(args.z_counts_per_mm),
    ]


def run_or_print(cmd: list[str], *, run: bool) -> None:
    print(" ".join(cmd), file=sys.stderr)
    if run:
        subprocess.run(cmd, check=True)


def main() -> int:
    args = parse_args()
    script_dir = Path(__file__).resolve().parent
    out_dir = Path(args.out_dir) if args.out_dir else Path(
        "experiment_series_" + time.strftime("%Y%m%d_%H%M%S")
    )
    out_dir.mkdir(parents=True, exist_ok=True)

    print(f"Output directory: {out_dir}", file=sys.stderr)
    if not args.yes:
        print("Dry run only. Re-run with --yes to move axes.", file=sys.stderr)

    py = sys.executable
    base = common_device_args(args)
    yes = ["--yes"] if args.yes else []
    allow_fast = ["--allow-fast-feedback"] if args.allow_fast_feedback else []

    if not args.skip_feedback_timing_probe:
        for sequence in [part for part in args.feedback_timing_sequences.split(",") if part]:
            timing_cmd = [
                py,
                str(script_dir / "feedback_timing_probe.py"),
                *base,
                "--feedback-period-ms",
                str(args.feedback_period_ms),
                *allow_fast,
                "--sequence",
                sequence,
                "--poll-count",
                str(args.feedback_timing_poll_count),
                "--trials",
                str(args.feedback_timing_trials),
                "--timeout-s",
                str(args.latency_timeout_s),
                "--out",
                str(out_dir / f"feedback_timing_{sequence}_{args.feedback_period_ms}ms.csv"),
            ]
            run_or_print(timing_cmd, run=args.yes)

    latency_cmd = [
        py,
        str(script_dir / "motion_latency_probe.py"),
        *yes,
        *base,
        "--axes",
        "both",
        "--length-mm",
        args.latency_length_mm,
        "--mode",
        "both",
        "--feed",
        str(args.latency_feed),
        "--slew",
        str(args.slew),
        "--trials",
        str(args.latency_trials),
        "--feedback-period-ms",
        str(args.feedback_period_ms),
        *allow_fast,
        "--safe-mm",
        str(args.safe_mm),
        "--timeout-s",
        str(args.latency_timeout_s),
        "--out",
        str(out_dir / f"latency_{args.feedback_period_ms}ms.csv"),
    ]
    if args.motion_feedback_timing:
        latency_cmd.append("--feedback-timing")
    run_or_print(latency_cmd, run=args.yes)

    interventions = [name for name in args.interventions.split(",") if name]
    directions = ["+", "-"]
    for index, experiment in enumerate(interventions):
        direction = directions[index % len(directions)]
        out = out_dir / (
            f"{index + 1:02d}_{experiment}_{args.intervention_axis}"
            f"{direction.replace('+', 'plus').replace('-', 'minus')}_"
            f"{args.intervention_length_mm:g}mm.csv"
        )
        cmd = [
            py,
            str(script_dir / "inflight_move_experiment.py"),
            *yes,
            *base,
            "--axis",
            args.intervention_axis,
            "--direction",
            direction,
            "--length-mm",
            str(args.intervention_length_mm),
            "--mode",
            "feed",
            "--feed",
            str(args.intervention_feed),
            "--slew",
            str(args.slew),
            "--feedback-period-ms",
            str(args.feedback_period_ms),
            *allow_fast,
            "--delay-us",
            str(args.intervention_delay_us),
            "--safe-mm",
            str(args.safe_mm),
            "--experiment",
            experiment,
            "--timeout-s",
            str(args.intervention_timeout_s),
            "--out",
            str(out),
        ]
        if args.intervention_feedback_timing:
            cmd.append("--feedback-timing")
        run_or_print(cmd, run=args.yes)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
