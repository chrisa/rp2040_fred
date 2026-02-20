from __future__ import annotations

import time

from fred_client import FredUsbClient


def main() -> None:
    client = FredUsbClient(0x2E8A, 0x000A)
    client.enable_polling(25)

    try:
        while True:
            print(client.refresh())
            time.sleep(0.05)
    except KeyboardInterrupt:
        pass
    finally:
        try:
            client.disable_polling()
        finally:
            client.close()


if __name__ == "__main__":
    main()
