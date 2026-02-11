RP2040 Firmware Outline (embassy-rp, Bus-Master Model)
=======================================================

Milestone
- Implement DRO-first FRED transport with RP2040 as active 1MHz-bus master.
- Drive:
  - `A0..A7`
  - shared `D0..D7`
  - `1MHZE`
  - `RnW`
  - `FRED_N`
  - `DATA_DIR`
  - `DATA_OE_N`

Protocol Target
- Maintain compatibility for legacy command cadence:
  - `03,02,01,00,07,06,05,04,0D,0C`
- Register semantics preserved:
  - command write (`FC80`)
  - status read (`FCF0`)
  - response read (`FCF1`)

Runtime Structure
- `main` task owns:
  - `DroProtocolEngine` (synthetic telemetry in bring-up)
  - `Rp2040FredTransport` (PIO-backed bus transaction engine)
- loop:
  1. consume command source
  2. produce status/response bytes
  3. drive bus transactions via PIO

PIO Integration Plan
1. Load `fred_bus_write` and `fred_bus_read` from `pio/fred_transport.pio`.
   - with contiguous mapping `GPIO0..7=D`, `GPIO8..15=A`.
2. Configure one SM for writes and one SM for reads (or separate read SM instances if needed).
3. Implement transaction helpers:
   - `write_fc80(cmd)`
   - `read_fcf0() -> u8`
   - `read_fcf1() -> u8`
   - include explicit read/write turn-around sequencing on shared data bus
4. Calibrate SM clock divider + NOPs to satisfy AN003 timing minima.

Bring-Up Checklist
1. Confirm static control pin polarity:
   - idle `FRED_N=1`, `RnW=1`, defined `1MHZE` idle state
2. Confirm write cycle timing against analyzer.
3. Confirm read sampling point (no data contention).
4. Verify DRO update behavior end-to-end.
