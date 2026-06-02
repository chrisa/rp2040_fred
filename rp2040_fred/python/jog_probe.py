#!/usr/bin/env python3

from __future__ import annotations

import argparse
import sys
import time
from typing import Dict, Iterable, Tuple

from fred_client import FredUsbClient


def generation_after(current: int, previous: int) -> bool:
    diff = (current - previous) & 0xFFFFFFFF
    return diff != 0 and diff < 0x80000000


def snapshot_generation(snapshot: Dict[str, object]) -> int:
    return int(snapshot.get("generation", 0))


def wait_snapshot(
    client: FredUsbClient,
    timeout_s: float,
    *,
    after_generation: int | None = None,
) -> Dict[str, object]:
    deadline = time.monotonic() + timeout_s

    while time.monotonic() < deadline:
        snapshot = client.refresh(timeout_ms=50)
        if snapshot is None:
            snapshot = client.latest_snapshot()
        if snapshot is None:
            continue

        generation = snapshot_generation(snapshot)
        if after_generation is None or generation_after(generation, after_generation):
            return snapshot

    raise TimeoutError("timed out waiting for telemetry snapshot")


def wait_idle_then_fresh_snapshot(
    client: FredUsbClient,
    timeout_s: float,
) -> Dict[str, object]:
    client.wait_idle(timeout_ms=int(timeout_s * 1000))
    latest = client.latest_snapshot()
    baseline_generation = snapshot_generation(latest) if latest is not None else 0
    return wait_snapshot(client, timeout_s, after_generation=baseline_generation)


def within_tolerance(
    snapshot: Dict[str, object],
    expected_x_mm: float,
    expected_z_mm: float,
    tolerance_mm: float,
) -> bool:
    return (
        abs(float(snapshot["x_mm"]) - expected_x_mm) <= tolerance_mm
        and abs(float(snapshot["z_mm"]) - expected_z_mm) <= tolerance_mm
    )


def wait_idle_then_target_snapshot(
    client: FredUsbClient,
    before: Dict[str, object],
    x_mm: float,
    z_mm: float,
    timeout_s: float,
    tolerance_mm: float,
) -> Dict[str, object]:
    client.wait_idle(timeout_ms=int(timeout_s * 1000))

    expected_x_mm = float(before["x_mm"]) + x_mm
    expected_z_mm = float(before["z_mm"]) + z_mm
    latest = client.latest_snapshot()
    baseline_generation = snapshot_generation(latest) if latest is not None else 0
    deadline = time.monotonic() + timeout_s
    last_snapshot = latest

    while time.monotonic() < deadline:
        snapshot = wait_snapshot(
            client,
            timeout_s=max(0.05, min(0.5, deadline - time.monotonic())),
            after_generation=baseline_generation,
        )
        baseline_generation = snapshot_generation(snapshot)
        last_snapshot = snapshot
        if within_tolerance(snapshot, expected_x_mm, expected_z_mm, tolerance_mm):
            return snapshot

    if last_snapshot is not None:
        print(
            "target wait timed out: "
            f"expected_x_mm={expected_x_mm:+.6f} "
            f"expected_z_mm={expected_z_mm:+.6f} "
            f"last_x_mm={float(last_snapshot['x_mm']):+.6f} "
            f"last_z_mm={float(last_snapshot['z_mm']):+.6f}",
            file=sys.stderr,
        )
        return last_snapshot

    raise TimeoutError("timed out waiting for target telemetry snapshot")


def snapshot_line(prefix: str, snapshot: Dict[str, object]) -> str:
    return (
        f"{prefix}: "
        f"gen={snapshot.get('generation')} tick={snapshot.get('tick')} "
        f"x_mm={float(snapshot['x_mm']):+.6f} "
        f"z_mm={float(snapshot['z_mm']):+.6f} "
        f"x_counts={int(snapshot['x_counts']):+d} "
        f"z_counts={int(snapshot['z_counts']):+d} "
        f"rpm={int(snapshot['spindle_rpm'])}"
    )


def observed_delta(
    before: Dict[str, object],
    after: Dict[str, object],
) -> Tuple[float, float, int, int]:
    return (
        float(after["x_mm"]) - float(before["x_mm"]),
        float(after["z_mm"]) - float(before["z_mm"]),
        int(after["x_counts"]) - int(before["x_counts"]),
        int(after["z_counts"]) - int(before["z_counts"]),
    )


def move_sequence(step_mm: float, axes: str) -> Iterable[Tuple[str, float, float]]:
    if axes in ("x", "both"):
        yield ("x+", step_mm, 0.0)
        yield ("x-", -step_mm, 0.0)
    if axes in ("z", "both"):
        yield ("z+", 0.0, step_mm)
        yield ("z-", 0.0, -step_mm)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Probe TCL125 jog sign/scale with direct Python FRED commands."
    )
    parser.add_argument("--yes", action="store_true", help="required; physically moves axes")
    parser.add_argument("--vid", type=lambda value: int(value, 0), default=0x2E8A)
    parser.add_argument("--pid", type=lambda value: int(value, 0), default=0x000A)
    parser.add_argument("--x-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--z-counts-per-mm", type=float, default=100.0)
    parser.add_argument("--poll-ms", type=int, default=10)
    parser.add_argument("--timeout-s", type=float, default=10.0)
    parser.add_argument("--step-mm", type=float, default=0.1)
    parser.add_argument("--axes", choices=("x", "z", "both"), default="both")
    parser.add_argument("--mode", choices=("feed", "rapid"), default="feed")
    parser.add_argument("--feed", type=int, default=100)
    parser.add_argument("--slew", type=int, default=61)
    parser.add_argument("--settle-s", type=float, default=0.1)
    parser.add_argument("--target-tolerance-mm", type=float, default=0.03)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    moves = list(move_sequence(args.step_mm, args.axes))

    print("Planned moves:")
    for label, x_mm, z_mm in moves:
        print(f"  {label}: x_mm={x_mm:+.6f} z_mm={z_mm:+.6f}")

    if not args.yes:
        print("Refusing to move without --yes.", file=sys.stderr)
        return 2

    with FredUsbClient(
        args.vid,
        args.pid,
        timeout_ms=int(args.timeout_s * 1000),
        x_counts_per_mm=args.x_counts_per_mm,
        z_counts_per_mm=args.z_counts_per_mm,
    ) as client:
        client.enable_polling(period_ms=args.poll_ms)
        try:
            before = wait_snapshot(client, args.timeout_s)
            print(snapshot_line("initial", before))

            for label, x_mm, z_mm in moves:
                before = wait_snapshot(
                    client,
                    args.timeout_s,
                    after_generation=snapshot_generation(before),
                )
                print(snapshot_line(f"{label} before", before))

                if args.mode == "feed":
                    sent = client.feed_move_delta(
                        x_mm=x_mm,
                        z_mm=z_mm,
                        feed=args.feed,
                        slew=args.slew,
                        wait=False,
                    )
                else:
                    sent = client.rapid_move_delta(
                        x_mm=x_mm,
                        z_mm=z_mm,
                        slew=args.slew,
                        wait=False,
                    )

                print(
                    f"{label} commanded: mode={args.mode} sent={sent} "
                    f"x_mm={x_mm:+.6f} z_mm={z_mm:+.6f}"
                )
                if not sent:
                    continue

                after = wait_idle_then_target_snapshot(
                    client,
                    before,
                    x_mm,
                    z_mm,
                    args.timeout_s,
                    args.target_tolerance_mm,
                )
                if args.settle_s > 0.0:
                    time.sleep(args.settle_s)
                    after = wait_snapshot(
                        client,
                        args.timeout_s,
                        after_generation=snapshot_generation(after),
                    )

                dx_mm, dz_mm, dx_counts, dz_counts = observed_delta(before, after)
                print(snapshot_line(f"{label} after", after))
                print(
                    f"{label} observed: "
                    f"dx_mm={dx_mm:+.6f} dz_mm={dz_mm:+.6f} "
                    f"dx_counts={dx_counts:+d} dz_counts={dz_counts:+d}"
                )

            return 0
        finally:
            client.disable_polling()


if __name__ == "__main__":
    raise SystemExit(main())
