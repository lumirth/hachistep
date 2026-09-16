# Inputs, outputs and replay

## Images

Firmware is a raw 49,152-byte internal image. External EEPROM is a raw 65,536-byte
array. The optional `--status` value contains only persistent M95512 status bits;
transient WIP/WEL values are not an initial persistent image. `--sensor-nv FILE`
loads the sensor's separate 19-byte nonvolatile configuration/calibration image.
All original files are read-only host inputs.

An exported EEPROM/status/sensor image set starts a new cold session. It does not
resume partial CPU or device operations. Use the in-memory snapshot API for that.
The CLI's `eeprom.status` export is binary, not the decimal text accepted by the
`--status` argument. For example, this Python snippet invokes a cold restart:

```python
from pathlib import Path
import subprocess
folder = Path("out-home")
status = (folder / "eeprom.status").read_bytes()
assert len(status) == 1
subprocess.run([
    "./target/release/hachistep", "run",
    "--firmware", "local-inputs/pokewalker.bin",
    "--eeprom", str(folder / "eeprom.bin"),
    "--status", str(status[0]),
    "--sensor-nv", str(folder / "sensor-nv.bin"),
    "--milliseconds", "1000", "--out", "out-restarted",
], check=True)
```

## Physical CSV

The first column is an unsigned integer microsecond timestamp from construction.
Blank lines and `#` comments are accepted. Records are ordered. At most one
assignment to each property is allowed at a timestamp. There is no header row.

| Record | Meaning |
|---|---|
| `1000,buttons,0,1,0` | Left released, center pressed, right released. |
| `2000,buttons,0,0,0` | Release all buttons. |
| `0,accel,0,0,1000000` | Specific force: +1 g on device Z. |
| `500000,supply,2900` | Change supply witness to 2,900 mV. |
| `100,ir,1` | Incident optical emission present. |
| `103,ir,0` | Incident emission absent. |
| `1000,reset,0` | Assert the MCU's active-low reset input. |
| `1010,reset,1` | Release reset. |

Button order is left, center, right. Values are logical physical pressed states,
not low-active line voltages; the reached Pokéwalker input path is active high.
The relevant package bits are PB2, PB0 and PB4 respectively. Debounce is firmware
behavior; a host may supply a clean or bouncing input history.

Acceleration is signed integer micro-g including gravity, with a +/-32 g safe
public input envelope. Values are held until the next record; **this starter uses
piecewise-constant inputs, not piecewise-linear interpolation**. The nominal
sensor samples the trajectory independently. A host's sparse recording cannot
recover physical information it never measured. The supplied walking CSV uses
100 Hz samples of a synthetic 2 Hz trajectory and does not inject steps.

The IR input is a digital optical-pulse witness interpreted by the receiver
model, not a socket packet and not a complete analog photodetector simulation.
The conformance IR fixture is an original software-generated pulse timeline.

## Exclusive timing

A record at the exact run endpoint is unprocessed. Use a later endpoint to
consume it. The authoritative time in reports/traces is the 64.64 raw integer.
Microsecond display conversion truncates; a human-readable time can display one
microsecond below a decimal input due to fixed-point rounding. That display is
not the core's ordering authority.

## Reports and traces

The CLI's `report.json` records schema/model identity, input image hashes, input
CSV hash, initial conditions, run partition, requested/current raw time,
register/continuation state, event counts, output hashes, status and fault.
Wall time is a host observation, not a deterministic guest value.

A trace line starts with the exact timestamp as 32 hexadecimal digits, followed
by a tab and a Rust diagnostic event. The trace text format is a development
format, not a stable cross-version interchange protocol. `--trace-limit` bounds
records written; `trace_dropped` and `trace_complete` expose truncation. Reaching
the cap never stops guest hardware. A bus trace needs the compile-time `trace`
feature and `--bus-trace` at runtime.

## Audio and display

To render captured buzzer drive without pretending a truncated trace is complete:

```sh
python3 tools/verify_retail.py --quick --menu-trace --out out/audio-run
python3 tools/render_audio.py --trace out/audio-run/menu-events.txt \
  --report out/audio-run/menu/report.json --out menu.wav
```

The WAV tool integrates differential-drive transitions over sample apertures.
It is an ideal-drive representation, not the frequency response or loudness of
the real piezo. It rejects missing/truncated traces and mismatched event counts.

`tools/preview.py` creates a standalone HTML viewer for captured PGM frames.
The viewer has no emulator, network access or live buttons; it displays the exact
supplied output pixels. The core is intended to be embedded by a real frontend.
