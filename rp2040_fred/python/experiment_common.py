from __future__ import annotations

import csv
import sys
import time
from pathlib import Path
from typing import IO, Dict, Iterable, Tuple

from fred_client import FredUsbClient


CSV_FIELDS = [
    "row_type",
    "host_time_ns",
    "trial_id",
    "phase",
    "axis",
    "direction",
    "mode",
    "length_mm",
    "feed",
    "slew",
    "feedback_period_ms",
    "script_name",
    "firmware_timestamp_us",
    "sample_index",
    "x_counts",
    "z_counts",
    "x_mm",
    "z_mm",
    "rpm",
    "flags",
    "op_index",
    "op_kind",
    "addr",
    "write_value",
    "read_value",
    "status",
    "event",
    "poll_index",
    "cmd_index",
    "cmd",
    "value",
    "total_us",
    "wait_before_us",
    "wait_after_us",
    "reads_before",
    "reads_after",
    "pending_records",
    "dropped_records",
    "active",
    "done",
    "error",
]

EVENT_NAMES = {
    1: "command_loaded",
    2: "command_complete",
    3: "error",
}


def monotonic_ns() -> int:
    return time.monotonic_ns()


def csv_output(path: str | None) -> Tuple[IO[str], csv.DictWriter]:
    if path is None or path == "-":
        writer = csv.DictWriter(sys.stdout, fieldnames=CSV_FIELDS)
        writer.writeheader()
        return sys.stdout, writer

    out = Path(path)
    fh = out.open("w", newline="")
    writer = csv.DictWriter(fh, fieldnames=CSV_FIELDS)
    writer.writeheader()
    return fh, writer


def base_row(row_type: str, meta: Dict[str, object]) -> Dict[str, object]:
    row = {field: "" for field in CSV_FIELDS}
    row["row_type"] = row_type
    row["host_time_ns"] = monotonic_ns()
    for key, value in meta.items():
        if key in row:
            row[key] = value
    return row


def write_status(
    writer: csv.DictWriter,
    status: Dict[str, object],
    meta: Dict[str, object],
    phase: str,
) -> None:
    row = base_row("status", meta)
    row["phase"] = phase
    row["flags"] = status.get("flags", "")
    row["pending_records"] = status.get("pending_records", "")
    row["dropped_records"] = status.get("dropped_records", "")
    row["active"] = status.get("active", "")
    row["done"] = status.get("done", "")
    row["error"] = status.get("error", "")
    writer.writerow(row)


def write_event(writer: csv.DictWriter, meta: Dict[str, object], phase: str) -> None:
    row = base_row("event", meta)
    row["phase"] = phase
    writer.writerow(row)


def write_record(
    writer: csv.DictWriter,
    record: Dict[str, object],
    meta: Dict[str, object],
    *,
    x_counts_per_mm: float,
    z_counts_per_mm: float,
) -> None:
    kind = str(record.get("kind", ""))
    row = base_row(kind, meta)
    row["firmware_timestamp_us"] = record.get("timestamp_us", "")
    row["flags"] = record.get("flags", "")
    row["status"] = record.get("status", "")

    if kind == "sample":
        x_counts = int(record["x_counts"])
        z_counts = int(record["z_counts"])
        row["sample_index"] = record.get("sample_index", "")
        row["x_counts"] = x_counts
        row["z_counts"] = z_counts
        row["x_mm"] = (x_counts * 2.0) / x_counts_per_mm
        row["z_mm"] = z_counts / z_counts_per_mm
        row["rpm"] = record.get("spindle_rpm", "")
    elif kind == "bus_op":
        row["op_index"] = record.get("op_index", "")
        row["op_kind"] = record.get("op_kind", "")
        row["addr"] = record.get("addr", "")
        row["write_value"] = record.get("write_value", "")
        row["read_value"] = record.get("read_value", "")
    elif kind == "event":
        event = int(record.get("event", 0))
        row["event"] = EVENT_NAMES.get(event, event)
    elif kind == "feedback_timing":
        row["poll_index"] = record.get("poll_index", "")
        row["cmd_index"] = record.get("cmd_index", "")
        row["cmd"] = record.get("cmd", "")
        row["value"] = record.get("value", "")
        row["total_us"] = record.get("total_us", "")
        row["wait_before_us"] = record.get("wait_before_us", "")
        row["wait_after_us"] = record.get("wait_after_us", "")
        row["reads_before"] = record.get("reads_before", "")
        row["reads_after"] = record.get("reads_after", "")

    writer.writerow(row)


def drain_experiment(
    client: FredUsbClient,
    writer: csv.DictWriter,
    meta: Dict[str, object],
    *,
    x_counts_per_mm: float,
    z_counts_per_mm: float,
    timeout_s: float,
) -> Dict[str, object]:
    deadline = time.monotonic() + timeout_s
    idle_since: float | None = None
    last_status: Dict[str, object] = {}

    while time.monotonic() < deadline:
        record = client.next_experiment_record(timeout_ms=50)
        if record is not None:
            write_record(
                writer,
                record,
                meta,
                x_counts_per_mm=x_counts_per_mm,
                z_counts_per_mm=z_counts_per_mm,
            )
            idle_since = None
            continue

        status = client.experiment_status()
        last_status = status
        if status.get("error"):
            write_status(writer, status, meta, "error")
            return status

        if not status.get("active") and int(status.get("pending_records", 0)) == 0:
            if idle_since is None:
                idle_since = time.monotonic()
            elif time.monotonic() - idle_since >= 0.1:
                write_status(writer, status, meta, "idle")
                return status
        else:
            idle_since = None

    write_status(writer, last_status, meta, "timeout")
    raise TimeoutError("experiment did not finish before timeout")


def parse_csv_floats(value: str) -> list[float]:
    return [float(part) for part in value.split(",") if part]


def parse_csv_ints(value: str) -> list[int]:
    return [int(part, 0) for part in value.split(",") if part]


def ensure_feedback_period(period_ms: int, allow_fast: bool) -> None:
    if period_ms <= 0:
        raise ValueError("feedback period must be greater than zero")
    if period_ms < 10 and not allow_fast:
        raise ValueError(
            "feedback periods below 10 ms require --allow-fast-feedback because fast polling can hang the controller"
        )


def assert_safe_sequence(
    moves: Iterable[Tuple[str, float, float]],
    safe_mm: float,
) -> None:
    x_offset = 0.0
    z_offset = 0.0
    for label, x_mm, z_mm in moves:
        x_offset += x_mm
        z_offset += z_mm
        if abs(x_offset) > safe_mm or abs(z_offset) > safe_mm:
            raise ValueError(
                f"planned move {label} exceeds software travel budget: x={x_offset:+.3f} z={z_offset:+.3f} safe={safe_mm:.3f}"
            )
