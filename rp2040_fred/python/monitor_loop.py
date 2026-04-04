from __future__ import annotations

import time

from fred_client import FredUsbClient


def main() -> None:
    client = FredUsbClient(0x2E8A, 0x000A)
    client.enable_capture()

    try:
        while True:
            samples = client.read_capture_samples(timeout_ms=50)
            for sample in samples:
                print(f"0x{sample:08X}")
            time.sleep(0.05)
    except KeyboardInterrupt:
        pass
    finally:
        try:
            client.disable_capture()
        finally:
            client.close()


if __name__ == "__main__":
    main()
