# Python USB Client

This is a pure-Python client for the RP2040 FRED USB bridge, using `pyusb`.

## Install

```bash
pip install pyusb
```

You also need a libusb backend installed on the host OS.

## Usage

```python
from fred_client import FredUsbClient

client = FredUsbClient(vid=0x2E8A, pid=0x000A)
client.enable_polling(period_ms=25)

snapshot = client.refresh()
print(snapshot)
# {
#   'x_mm': ...,
#   'z_mm': ...,
#   'spindle_rpm': ...,
#   'x_counts': ...,
#   'z_counts': ...,
#   'tick': ...,
#   'flags': ...,
# }

client.disable_polling()
client.close()
```

`enable_polling()` now sends `CAPTURE_SET=0` first, because firmware powers up in passive capture mode.

You can toggle capture explicitly:

```python
client.enable_capture()
client.disable_capture()
```

`refresh()` drains any pending telemetry packets and returns the latest snapshot.
