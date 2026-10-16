# CLI inputs and outputs

Run `hachistep run --help` for the full option list.

## Images

| Input | Format |
| --- | --- |
| `--firmware FILE` | Raw 49,152-byte internal flash, including vectors. |
| `--eeprom FILE` | Raw 65,536-byte external EEPROM. |
| `--status VALUE` | Persistent M95512 status bits, decimal or `0x` hexadecimal. Defaults to zero. |
| `--sensor-nv FILE` | Optional 19-byte BMA150 configuration and calibration image. |

Store private images under ignored `inputs/` or pass paths directly.

Use [a save state](SAVE_STATES.md) to resume a running session. Loading exported
persistent images starts a new session.

## Input CSV

`--input FILE` accepts ordered records with absolute integer microsecond timestamps.
There is no header. Blank lines and `#` comments are accepted. Assign each property at
most once per timestamp. Values remain in effect until the next assignment.

| Example | Meaning |
| --- | --- |
| `1000,buttons,0,1,0` | Left, center, right pressed flags. |
| `2000,buttons,0,0,0` | Release the buttons. |
| `0,accel,0,0,1000000` | X, Y, Z specific force in signed micro-g, including gravity. |
| `0,temperature,20000` | Temperature in millidegrees Celsius. |
| `500000,supply,2900` | Board supply in millivolts. |
| `500000,power,0` | Disconnect the battery. Use `1` to reconnect. |
| `100,ir,1` | Incident optical emission. Use `0` for its absence. |
| `1000,reset,0` | Assert active-low RES. Use `1` to drive it high. |
| `100,nmi,0` | Drive NMI low. Its initial level is high. |
| `100,analog,pb4,1900` | Drive an analog package pin in millivolts. |
| `300,analog,pb4,release` | Release that fixture drive. |
| `100,digital,p11,1` | Drive a digital package pin. Values are `0`, `1`, or `release`. |

Acceleration accepts +/-32 g. Analog fixture names are `pb0` through `pb5` and `vcref`.
Digital fixture names are `p10` through `p12`, `p30` through `p32`, `p90` through `p93`,
and `adtrg`. Holding NMI low at reset release can select an unsupported boot mode.

`--milliseconds` is an absolute, exclusive endpoint. An input exactly at that endpoint
belongs to the next run. After `--load-state`, earlier CSV records are skipped.
The report's raw 64.64 time preserves exact ordering; displayed microseconds truncate.

## Exports

`--out NEW_DIRECTORY` writes `frame.pgm`, flash and EEPROM images, EEPROM status,
sensor nonvolatile data, LCD data, `state.bin`, and `report.json`. Reports include input
identities, reached time, counters, output hashes, and any fault. Output files and
directories must be new.

`eeprom.status` contains one binary byte. Pass its numeric value to `--status` when
starting from the exported persistent images. A snapshot also preserves operations in
progress, including partially programmed cells.

`--trace NEW_FILE` records timestamped product events. `--trace-limit` bounds the
record count; check `trace_complete` before using a trace as a complete history.
`--bus-trace` additionally requires a build with the `trace` feature. Trace text is a
development format and can change between revisions.

Capture audio from the core renderer:

```sh
./target/release/hachistep run --firmware firmware.bin --eeprom eeprom.bin \
  --input inputs.csv --milliseconds 6500 --out out/menu --audio out/menu.wav
```

The WAV contains mono 48 kHz signed PCM16. `--write-ram f78e:0064` edits RAM before
execution; repeat the option for additional edits. Addresses and byte strings are
hexadecimal. Edits use the same [debugging API](API.md#persistence-and-editing) as an
embedding caller.

## Linked devices

```sh
./target/release/hachistep link --firmware firmware.bin --eeprom first.bin \
  --peer-eeprom second.bin --input first.csv --peer-input second.csv \
  --delay-us 1 --milliseconds 12000 --out out/link
```

Both devices start at time zero with the same firmware and their respective EEPROM
images and input timelines. Each optical emission reaches the other device after
`--delay-us` microseconds. The channel delay also bounds run-ahead, preserving pulse
ordering. The peer's exports are under `out/link/peer/`.

`tools/preview.py` creates an HTML viewer for captured PGM frames. Frontends can use
the [display and audio APIs](API.md) directly.
