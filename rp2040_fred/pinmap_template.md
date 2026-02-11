RP2040 Pin Map Template (Bus-Master FRED)
=========================================

Fill this before wiring `transport.rs` PIO configuration.

Address Bus (A0..A7)
- `A0  -> GPIO8`
- `A1  -> GPIO9`
- `A2  -> GPIO10`
- `A3  -> GPIO11`
- `A4  -> GPIO12`
- `A5  -> GPIO13`
- `A6  -> GPIO14`
- `A7  -> GPIO15`

Shared Data Bus (bidirectional via transceiver)
- `D0 <-> GPIO0`
- `D1 <-> GPIO1`
- `D2 <-> GPIO2`
- `D3 <-> GPIO3`
- `D4 <-> GPIO4`
- `D5 <-> GPIO5`
- `D6 <-> GPIO6`
- `D7 <-> GPIO7`

Control Outputs
- `1MHZE   -> GPIO17`
- `RnW     -> GPIO16`
- `FRED_N  -> GPIO20`
- `DATA_OE_N -> GPIO28`
- `DATA_DIR  -> GPIO27`

PIO Allocation
- `PIO0 SM0`: `fred_bus_write`
- `PIO0 SM1`: `fred_bus_read`

Clocking
- Target PIO clock divider: `________`
- Expected write-cycle length: `________ ns`
- Expected read-cycle length: `________ ns`

Notes
- This map intentionally places `D0..D7` on GPIO0..7 and `A0..A7` on GPIO8..15,
  so PIO can emit both with one `out pins, 16` operation.
- Shared data bus direction is controlled by `DATA_DIR` and `DATA_OE_N`.
