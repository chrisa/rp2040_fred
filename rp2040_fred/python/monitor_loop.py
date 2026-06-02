from __future__ import annotations

from fred_client import FredUsbClient


def main() -> None:
    client = FredUsbClient(0x2E8A, 0x000A)
    client.enable_polling(period_ms=10, rpm_service="manual")

    try:
        while True:
            print(client.next_snapshot())
    except KeyboardInterrupt:
        pass
    finally:
        try:
            client.disable_polling()
        finally:
            client.close()


if __name__ == "__main__":
    main()
