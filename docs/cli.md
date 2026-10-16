# Run firmware with the CLI

To be clear, the CLI isn't exactly built to be human-friendly. It mostly exists to allow a programmatic interface for agents. It should still be functional enough for real people to tinker with.

The CLI runs a firmware image against a timeline of physical inputs. The captures
let you inspect where the device ended up, then resume it or repeat the experiment
with a change to the firmware.

Build the [release CLI](build.md) first. The commands below run from the hachistep
checkout; `hachistep run --help` gives the full option list.

## Start a session

Supply a raw 49,152-byte internal flash image and a 65,536-byte external EEPROM image:

```sh
mkdir -p out
./target/release/hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --milliseconds 10000 --out out/session
```

This advances the device to ten seconds. `--milliseconds` is an absolute endpoint,
which also matters when resuming a capture later. `--out` saves the final screen,
persistent images, `state.bin`, and `report.json`. The report records the reached
time, execution counters, input identities, and any fault.

Create the parent directory first and choose new paths for output files and
directories. Open `out/session/frame.pgm` to inspect the screen, or make a browser
preview:

```sh
python3 tools/preview.py out/session/frame.pgm --out out/session.html
```

Open `out/session.html` in a browser. The image records the controller's programmed
pixel drive; the preview gives it a display tint.

## Supply physical inputs

Inputs describe what happens to the device. This CSV presses the center button at
one second and releases it at 1.1 seconds. Times are absolute microseconds:

```csv
1000000,buttons,0,1,0
1100000,buttons,0,0,0
```

Save it as `inputs.csv`, then run:

```sh
./target/release/hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --input inputs.csv --milliseconds 10000 --out out/buttons
```

Keep records in timestamp order, without a header. Blank lines and `#` comments
are accepted. Each value remains in effect until changed, with at most one assignment
to a property at each timestamp. An input exactly at the run's endpoint belongs to
the next run.

### Input reference

| Record | Meaning |
| --- | --- |
| `1000,buttons,0,1,0` | Left, center, right pressed flags. |
| `2000,buttons,0,0,0` | Release all buttons. |
| `0,accel,0,0,1000000` | X, Y, Z specific force in signed micro-g, including gravity. Range is +/-32 g. |
| `0,temperature,20000` | Temperature in millidegrees Celsius. |
| `500000,supply,2900` | Board supply in millivolts. |
| `500000,power,0` | Disconnect the battery; `1` reconnects it. |
| `100,ir,1` | Incident optical emission; `0` removes it. |
| `1000,reset,0` | Assert active-low RES; `1` drives it high. |
| `100,nmi,0` | Drive NMI low. Its initial level is high. |
| `100,analog,pb4,1900` | Drive an analog package pin in millivolts. |
| `300,analog,pb4,release` | Release the analog fixture drive. |
| `100,digital,p11,1` | Drive a digital package pin; values are `0`, `1`, or `release`. |

Analog fixture names are `pb0` through `pb5` and `vcref`. Digital fixture names are
`p10` through `p12`, `p30` through `p32`, `p90` through `p93`, and `adtrg`.
Holding NMI low at reset release can select an unsupported manufacturer boot mode.

## Capture audio and traces

Add `--audio` to write mono 48 kHz signed PCM16 audio:

```sh
./target/release/hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --input inputs.csv --milliseconds 10000 --out out/audio --audio out/session.wav
```

For timestamped device events, add `--trace NEW_FILE`. Set `--trace-limit` to bound
the record count and check `trace_complete` in the report to see whether the history
fit. CPU bus accesses also need a [trace build](build.md#trace-bus-accesses) and
`--bus-trace`. The text format can change between revisions.

## Continue a saved session

Every `--out` capture contains `state.bin`. Resume the first example and advance to
twenty seconds:

```sh
./target/release/hachistep run --load-state out/session/state.bin \
  --milliseconds 20000 --out out/continued
```

The device resumes at ten seconds and runs to twenty. You can supply the same input
CSV again; the CLI skips records before the restored time. The state includes the
device's images and initial conditions, and needs the matching hachistep revision.

`--save-state NEW_FILE` saves a state without requesting the whole output directory.
A state preserves work in progress, such as a partly completed EEPROM write.
Loading exported EEPROM and firmware images starts a fresh device using those bytes.

## Connect two devices

```sh
./target/release/hachistep link --firmware firmware.bin --eeprom first.bin \
  --peer-eeprom second.bin --input first.csv --peer-input second.csv \
  --delay-us 1 --milliseconds 12000 --out out/link
```

Each device starts at time zero, using the same firmware but its own EEPROM and
input timeline. `--delay-us` is the optical propagation delay in microseconds. It
also limits how far a device can run ahead, so pulses reach its peer in order.
Look under `out/link/peer/` for the second device's captures.

## Initial conditions and inspection

| Option | Purpose |
| --- | --- |
| `--status VALUE` | Load persistent M95512 status bits, decimal or `0x` hexadecimal. Defaults to zero. |
| `--sensor-nv FILE` | Load a 19-byte BMA150 configuration and calibration image. |
| `--supply-mv VALUE` | Set the initial board supply. Defaults to 3000 mV. |
| `--avcc-mv VALUE` | Supply an external ADC-reference fixture. The default follows the board rail. |
| `--battery-drop-mv VALUE` | Set the effective battery-sense drop. Defaults to 600 mV. |
| `--peek ADDRESS` | Inspect a hexadecimal address after execution without guest side effects. Repeatable. |
| `--write-ram ADDRESS:HEX` | Edit RAM before execution, for example `--write-ram f78e:0064`. Repeatable. |
| `--chunk-us VALUE` | Bound each host execution call to this many microseconds. |

`eeprom.status` in an output directory contains one binary byte. Supply its numeric
value with `--status` when starting a new session from exported persistent images.
Store private images in ignored `inputs/` or pass paths directly.

Reports preserve exact time as raw unsigned 64.64 seconds. The microsecond display
truncates that value. Before using RAM edits, see the
[debugging contract](api.md#debugging) for how they interact
with instructions and writes already in progress.
