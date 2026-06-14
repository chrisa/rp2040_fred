# TCL125 Hall GPIO Interface

## Sensor Type

Preferred sensor style: 3-wire Hall switch such as Multicomp Pro
`MP-HS-324-02-0300`.

Relevant datasheet points:

- `red`: supply voltage (`VSUP`)
- `white`: open-drain output (`OUT`)
- `black`: ground (`GND`)
- recommended supply range includes `2.7 V` and up, so `3.3 V` operation is
  valid for a Pi GPIO interface
- output low voltage is specified while sinking current, so the signal should be
  treated as an open-drain line with a pull-up
- the shown switching type is latching, so magnet layout must provide reliable
  set and release magnetic polarity

## Recommended Electrical Interface

Use a 3.3 V-only interface:

```text
Pi 3V3  ----+---------------- red / VSUP
            |
            +-- 4.7k --+----- white / OUT ---- 220R..1k ---- Pi GPIO input
                       |
Hall black / GND ------+----------------------- Pi GND
```

Default values:

- external pull-up: `4.7 kΩ` to Pi `3.3 V`
- optional series GPIO resistor: `220 Ω` to `1 kΩ`
- decoupling near connector board: `100 nF` per sensor group plus `4.7-10 uF`
  bulk

Do not pull outputs up to 5 V. If a Hall sensor is powered from a voltage above
3.3 V, the output pull-up must still be to 3.3 V and the exact part behavior
must be verified before connecting it to the Pi. The simple supported design is
sensor `VSUP = 3.3 V`.

The software default is `active_low = true`: inactive output is pulled high,
active output is pulled low by the Hall switch.

## Mechanical Approach

For linear axes, use the datasheet's slide-by magnetic approach rather than
frontal or rotating approaches.

Because the chosen part is latching:

- verify the actual set and release edges with the real magnet, air gap, and
  bracket geometry
- use a two-pole magnet arrangement, or separate set/release pole geometry, if a
  single magnet does not reliably unlatch at the intended release position
- record the measured leading edge used as the homing datum
- keep the software polarity configurable

## Host Software

Rust uses Linux GPIO character devices through the `gpiod` crate.

Diagnostic command:

```sh
cargo run --manifest-path host/Cargo.toml -- \
  home gpio-monitor \
  --chip gpiochip0 \
  --x-coarse 17 --x-home 27 \
  --z-coarse 22 --z-home 23
```

Options:

- `--chip <gpiochipN>` selects the GPIO chip, default `gpiochip0`
- `--active-low` is the default and matches the open-drain pull-up interface
- `--active-high` is available for bench tests or alternate hardware
- `--poll-ms <ms>` controls how often the diagnostic loop drains queued events

The GPIO layer records:

- logical states for `x_coarse`, `x_home`, `z_coarse`, `z_home`
- both-edge events
- kernel event timestamp, as exposed by `gpiod`

The homing backend should use this GPIO layer while each autonomous movement is
running, then include drained events in the same `MoveObservation` record as the
controller telemetry.
