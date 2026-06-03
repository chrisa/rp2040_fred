"""Python compatibility wrapper for the Rust-backed FRED monitor client."""

from __future__ import annotations

from typing import Dict, Iterable, Optional

from ._fred_native import FredProtocolError, FredUsbError
from ._fred_native import FredUsbClient as _NativeFredUsbClient

_BUS_OP_KIND = {
    "delay_us": 0x01,
    "read": 0x02,
    "write": 0x03,
    "write_gated": 0x04,
    "read_until": 0x05,
    "read_status_until": 0x05,
    "poll_feedback_once": 0x06,
}


class FredUsbClient:
    """RP2040 FRED USB client.

    This preserves the historical Python API for monitor/telemetry use while
    delegating the implementation to the Rust monitor client.
    """

    def __init__(
        self,
        vid: int,
        pid: int,
        *,
        timeout_ms: int = 1000,
        x_counts_per_mm: float = 100.0,
        z_counts_per_mm: float = 100.0,
    ) -> None:
        self.vid = vid
        self.pid = pid
        self.timeout_ms = timeout_ms
        self.x_counts_per_mm = x_counts_per_mm
        self.z_counts_per_mm = z_counts_per_mm
        self._inner = _NativeFredUsbClient(
            vid,
            pid,
            timeout_ms=timeout_ms,
            x_counts_per_mm=x_counts_per_mm,
            z_counts_per_mm=z_counts_per_mm,
        )

    def close(self) -> None:
        self._inner.close()

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def __enter__(self) -> "FredUsbClient":
        return self

    def __exit__(self, exc_type, exc, tb) -> bool:
        self.close()
        return False

    def enable_polling(self, period_ms: int = 25, rpm_service: str = "manual") -> None:
        self._inner.enable_polling(period_ms=period_ms, rpm_service=rpm_service)

    def disable_polling(self) -> None:
        self._inner.disable_polling()

    def enable_capture(self) -> None:
        raise NotImplementedError("Passive capture is not exposed in the Rust-backed Python client")

    def disable_capture(self) -> None:
        raise NotImplementedError("Passive capture is not exposed in the Rust-backed Python client")

    def refresh(self, timeout_ms: int = 0) -> Optional[Dict[str, object]]:
        snapshot = self._inner.refresh_timeout(timeout_ms=timeout_ms)
        if snapshot is None:
            return None
        return dict(snapshot)

    def next_snapshot(self) -> Dict[str, object]:
        return dict(self._inner.next_snapshot())

    def latest_snapshot(self) -> Optional[Dict[str, object]]:
        snapshot = self._inner.latest_snapshot()
        if snapshot is None:
            return None
        return dict(snapshot)

    def rapid_move_delta(
        self,
        *,
        x_mm: float = 0.0,
        z_mm: float = 0.0,
        slew: int = 61,
        wait: bool = False,
    ) -> bool:
        return bool(self._inner.rapid_move_delta(x_mm=x_mm, z_mm=z_mm, slew=slew, wait=wait))

    def feed_move_delta(
        self,
        *,
        x_mm: float = 0.0,
        z_mm: float = 0.0,
        feed: int = 100,
        slew: int = 61,
        wait: bool = False,
    ) -> bool:
        return bool(
            self._inner.feed_move_delta(
                x_mm=x_mm,
                z_mm=z_mm,
                feed=feed,
                slew=slew,
                wait=wait,
            )
        )

    def controller_status(self) -> Dict[str, object]:
        return dict(self._inner.controller_status())

    def run_experiment_move_delta(
        self,
        *,
        x_mm: float = 0.0,
        z_mm: float = 0.0,
        mode: str = "rapid",
        feed: int = 100,
        slew: int = 61,
        feedback_period_ms: int = 10,
        trial_id: int = 0,
        script_ops: Optional[Iterable[Dict[str, int | str]]] = None,
        feedback_timing: bool = False,
    ) -> bool:
        return bool(
            self._inner.run_experiment_move_delta(
                x_mm=x_mm,
                z_mm=z_mm,
                mode=mode,
                feed=feed,
                slew=slew,
                feedback_period_ms=feedback_period_ms,
                trial_id=trial_id,
                script_ops=_script_ops_to_native(script_ops or ()),
                feedback_timing=feedback_timing,
            )
        )

    def run_feedback_timing_experiment(
        self,
        *,
        feedback_period_ms: int = 10,
        trial_id: int = 0,
        poll_count: int = 30,
        sequence: str = "full",
    ) -> None:
        self._inner.run_feedback_timing_experiment(
            feedback_period_ms=feedback_period_ms,
            trial_id=trial_id,
            poll_count=poll_count,
            sequence=sequence,
        )

    def next_experiment_record(self, timeout_ms: int = 0) -> Optional[Dict[str, object]]:
        record = self._inner.next_experiment_record(timeout_ms=timeout_ms)
        if record is None:
            return None
        return dict(record)

    def experiment_status(self) -> Dict[str, object]:
        return dict(self._inner.experiment_status())

    def wait_experiment_idle(self, timeout_ms: Optional[int] = None) -> None:
        self._inner.wait_experiment_idle(timeout_ms=timeout_ms)

    def wait_idle(self, timeout_ms: Optional[int] = None) -> None:
        self._inner.wait_idle(timeout_ms=timeout_ms)

    def set_spindle(
        self,
        *,
        on: bool,
        rpm: float = 0.0,
        forward: bool = True,
        ssl: Optional[int] = None,
        wait: bool = False,
    ) -> bool:
        return bool(
            self._inner.set_spindle(
                on=on,
                rpm=rpm,
                forward=forward,
                ssl=ssl,
                wait=wait,
            )
        )

    def change_tool(
        self,
        *,
        current_station: int,
        target_station: int,
        slew: int = 61,
        wait: bool = False,
    ) -> bool:
        return bool(
            self._inner.change_tool(
                current_station=current_station,
                target_station=target_station,
                slew=slew,
                wait=wait,
            )
        )

    def read_capture_samples(self, timeout_ms: int = 1) -> list[int]:
        raise NotImplementedError(
            "Passive capture is not exposed in the Rust-backed Python client"
        )


def _script_ops_to_native(
    ops: Iterable[Dict[str, int | str]],
) -> list[tuple[int, int, int, int, int, int]]:
    native: list[tuple[int, int, int, int, int, int]] = []
    for op in ops:
        name = str(op.get("op", ""))
        try:
            kind = _BUS_OP_KIND[name]
        except KeyError as exc:
            raise ValueError(f"unknown experiment bus op: {name}") from exc

        addr = int(op.get("addr", 0))
        value = int(op.get("value", 0))
        mask = int(op.get("mask", 0))
        match_value = int(op.get("match_value", op.get("value", 0)))
        arg_us = int(op.get("arg_us", op.get("delay_us", op.get("timeout_us", 0))))
        native.append((kind, addr, value, mask, match_value, arg_us))
    return native
