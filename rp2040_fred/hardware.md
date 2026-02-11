RP2040 1MHz Bus Master Interface (BBC Replacement)
===================================================

Purpose
- RP2040 replaces BBC-side FRED accesses and actively drives the external 1MHz bus signals.
- DRO-first target still uses the same logical register accesses:
  - write command to `FC80`
  - read status from `FCF0`
  - read response from `FCF1`

Reference
- Acorn AN003 (1MHz bus timing).
- Key constraints to respect:
  - `tas >= 300ns`, `tah >= 30ns`
  - `tcs >= 250ns`, `tch >= 30ns`
  - write `tdsw <= 150ns`, `tdhw >= 50ns`
  - read `tdsr >= 200ns`, `tdhr >= 30ns`

Driven Signals (RP2040 -> Bus)
- `A[7:0]` (lower address byte only)
- `D[7:0]` (shared bidirectional bus; driven only during write phase)
- `1MHZE`
- `RnW`
- `FRED_N` (predecoded FRED select, active low)

Observed/Read Signals (Bus -> RP2040)
- `D[7:0]` during read cycles (same shared GPIO group via transceiver direction switch).

Important Scope Clarification
- Only lower address byte is driven by RP2040 (`A0..A7`) as requested.
- Upper-page selection is not encoded on `A[15:8]`; `FRED_N` is explicitly driven as a dedicated control line.

Electrical Requirements
- 5V compatibility:
  - RP2040 GPIO are not 5V tolerant.
  - Use level-shifting/bus-transceiver devices for both address/data/control directions.
- Data-bus direction:
  - explicit transceiver direction + OE control required.
  - on read cycles, RP2040 must release bus drive before sampling.

Proposed Logical Pin Groups
- `ADDR[7:0]`  : output group
- `DATA[7:0]` : single shared GPIO group (bidirectional through transceiver)
- control outputs:
  - `PIN_1MHZE`
  - `PIN_RNW`
  - `PIN_FRED_N`
  - `PIN_DATA_OE_N`
  - `PIN_DATA_DIR` (RP2040->bus = write, bus->RP2040 = read)

PIO Strategy
- Use two programs:
  - `fred_bus_write`: consumes `(addr_lo, data)` and performs one write cycle.
  - `fred_bus_read`: consumes `addr_lo`, performs one read cycle, pushes read byte.
- PIO side-set also controls transceiver `DATA_DIR` and `DATA_OE_N`.
- Preferred mapping is contiguous `GPIO0..15` with:
  - `GPIO0..7 = D0..D7`
  - `GPIO8..15 = A0..A7`
  so address+data can be emitted in one `out pins, 16`.
- Timing is set by PIO clock divider and optional NOP padding.
- Keep cycle shaping in PIO for deterministic nanosecond-level behavior.

Cycle Model (initial)
- Write:
  1. drive `A0..A7`, drive `D0..D7`, set `RnW=0`, `FRED_N=0`
  2. toggle/hold `1MHZE` active phase
  3. return idle (`FRED_N=1`, `RnW=1`)
- Read:
  1. drive `A0..A7`, set `RnW=1`, `FRED_N=0`, release data drive
  2. toggle/hold `1MHZE` active phase
  3. sample `D0..D7`, push to RX FIFO
  4. return idle

Bring-Up Sequence
1. Static pin validation:
  - confirm address/control pin polarity and idle states.
2. Write-cycle validation:
  - issue repeated writes to `FC80` and scope setup/hold vs 1MHZE.
3. Read-cycle validation:
  - read `FCF0/FCF1` and verify sample point and no bus contention.
4. Protocol validation:
  - run DRO cadence (`03,02,01,00,07,06,05,04,0D,0C`) and confirm expected display behavior.
