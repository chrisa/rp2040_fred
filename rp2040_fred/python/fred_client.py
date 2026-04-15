"""Python compatibility wrapper for the Rust-backed FRED monitor client."""

from __future__ import annotations

from typing import Dict

from _fred_native import FredProtocolError, FredUsbError
from _fred_native import FredUsbClient as _NativeFredUsbClient


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
        timeout_ms: int = 250,
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

    def enable_polling(self, period_ms: int = 25) -> None:
        self._inner.enable_polling(period_ms=period_ms)

    def disable_polling(self) -> None:
        self._inner.disable_polling()

    def enable_capture(self) -> None:
        raise NotImplementedError("Passive capture is not exposed in the Rust-backed Python client")

    def disable_capture(self) -> None:
        raise NotImplementedError("Passive capture is not exposed in the Rust-backed Python client")

    def refresh(self) -> Dict[str, object]:
        return dict(self._inner.refresh())

    def read_capture_samples(self, timeout_ms: int = 1) -> list[int]:
        raise NotImplementedError(
            "Passive capture is not exposed in the Rust-backed Python client"
        )
