# Inputs, outputs and replay

## Images

Create ignored `inputs/` for private firmware, EEPROM saves and calibration images, or
pass their paths directly. Tool defaults use `inputs/pokewalker.bin` and
`inputs/eeprom.bin`. Keep generated exports, reports, traces and rendered media under
ignored `out/`. The tracked `workloads/` directory contains reusable input timelines
and reviewed retail regression expectations.

Firmware is a raw 49,152-byte internal image. External EEPROM is a raw 65,536-byte
array. The optional `--status` value contains only persistent M95512 status bits;
transient WIP/WEL values are not an initial persistent image. `--sensor-nv FILE` loads
the sensor's separate 19-byte nonvolatile configuration/calibration image. All original
files are read-only host inputs.

An exported EEPROM/status/sensor image set starts a new cold session. It does not resume
partial CPU or device operations. Use `--load-state FILE` or the snapshot API to resume
those. `--out` includes a native `state.bin`; `--save-state FILE` can save one
separately. Run endpoints and CSV timestamps remain absolute after a load. Records
earlier than the saved instant are skipped; an input exactly at that instant remains
pending. Exports project persistent cells at the current time. During programming they
can contain a partially erased/programmed value; exporting does not complete or cancel
the operation. A snapshot also retains the operation needed to continue. The CLI's
`eeprom.status` export is binary, not the decimal text accepted by the `--status`
argument. For example, this Python snippet invokes a cold restart:

```python
from pathlib import Path
import subprocess
folder = Path("out/home")
status = (folder / "eeprom.status").read_bytes()
assert len(status) == 1
subprocess.run([
    "./target/release/hachistep", "run",
    "--firmware", "inputs/pokewalker.bin",
    "--eeprom", str(folder / "eeprom.bin"),
    "--status", str(status[0]),
    "--sensor-nv", str(folder / "sensor-nv.bin"),
    "--milliseconds", "1000", "--out", "out/restarted",
], check=True)
```

## Physical CSV

The first column is an unsigned integer microsecond timestamp from construction. Blank
lines and `#` comments are accepted. Records are ordered. At most one assignment to each
property is allowed at a timestamp. There is no header row.

| Record | Meaning |
|---|---|
| `1000,buttons,0,1,0` | Left released, center pressed, right released. |
| `2000,buttons,0,0,0` | Release all buttons. |
| `0,accel,0,0,1000000` | Specific force: +1 g on device Z. |
| `0,temperature,20000` | Sensor temperature in millidegrees Celsius: 20 °C. |
| `500000,supply,2900` | Change board supply to 2,900 mV. |
| `500000,supply,0` | Collapse the board rail; stop activity and retain partial nonvolatile writes. |
| `600000,supply,3000` | Restore the rail after a zero-voltage interval. |
| `500000,power,0` | Remove board power at the current configured voltage. |
| `600000,power,1` | Reconnect board power. |
| `100,ir,1` | Incident optical emission present. |
| `103,ir,0` | Incident emission absent. |
| `1000,reset,0` | Assert the MCU's active-low reset input. |
| `1010,reset,1` | Drive RES high; internal release follows eight reference-clock edges. |
| `100,nmi,0` | Drive NMI low; the selected edge is latched even while CCR.I is set. |
| `300,nmi,1` | Drive NMI high (also the default user-mode reset strap). |
| `100,analog,pb4,1900` | Apply 1,900 mV to a comparator/ADC fixture node. |
| `200,analog,vcref,1200` | Apply external comparator reference at VCref/P30. |
| `300,analog,pb4,release` | Release the external analog fixture driver. |
| `100,digital,p11,1` | Drive an AEC/Timer W-related package input high. |
| `300,digital,p11,release` | Release that digital fixture driver. |

Button order is left, center, right. A value of 1 means pressed; the Pokéwalker input
path is active high. The relevant package bits are PB2, PB0 and PB4 respectively.
Debounce is firmware behavior; a host may supply a clean or bouncing input history.

Acceleration is signed integer micro-g including gravity, with a +/-32 g safe public
input envelope. Values are held until the next record, forming a piecewise-constant
trajectory. The nominal sensor samples the trajectory independently. A host's sparse
recording cannot recover physical information it never measured. The supplied walking
CSV uses 100 Hz samples of a synthetic 2 Hz trajectory and does not inject steps.

The IR input supplies optical levels and their timing to the receiver model. The
conformance IR fixture generates a pulse timeline in software.

`tools/verify_retail.py` also runs two independent retail machines through a peer
exchange. It prepares distinct identities and empty encounter histories in memory from
a save with a walking Pokémon. Button presses start the connection; each machine's
emitted pulses reach the other after a selected one-microsecond channel delay. The test
checks that both firmwares commit the other device's record to encounter history.
The delay defines this simulated channel; it is not a measurement of the physical
transceiver. Source save files remain read-only.

Analog fixture names are `pb0` through `pb5` and `vcref`; values are integer mV within
the checked input envelope. Digital fixture names are `p10`–`p12`, `p30`–`p32`,
`p90`–`p93`, and `adtrg`, with `0`, `1`, or `release`. They drive input or released
open-drain nodes through the selected pin functions. These fixtures represent electrical
connections to the package pads.

NMI starts high. IEGR bit 7 selects its edge. A low level at actual RES release selects
manufacturer boot mode when TEST/ADTRG is low, with E7_0 assumed high. TEST/ADTRG high
selects a test mode. Both are unsupported and return a host diagnostic. A retained
supply dip without RES assertion does not sample a new reset strap. Held levels do not
continuously reassert NMI. Short-pulse/subcycle synchronizer behavior remains
uncharacterized.

## Exclusive timing

A record at the exact run endpoint is unprocessed. Use a later endpoint to consume it.
The authoritative time in reports/traces is the 64.64 raw integer. Microsecond display
conversion truncates; a human-readable time can display one microsecond below a decimal
input due to fixed-point rounding. That display is not the core's ordering authority.

## Reports and traces

The CLI's `report.json` records schema/model identity, input image hashes, input CSV
hash, initial conditions, run partition, requested/current raw time,
register/continuation state, event counts, output hashes, status and fault. Wall time is
a host observation, not a deterministic guest value.

A trace line starts with the exact timestamp as 32 hexadecimal digits, followed by a tab
and a Rust diagnostic event. The trace text format is a development format, not a stable
interchange protocol across versions. `--trace-limit` bounds records written;
`trace_dropped` and `trace_complete` expose truncation. Reaching the cap never stops
guest hardware. A bus trace needs the compile-time `trace` feature and `--bus-trace` at
runtime.

## Audio and display

For streaming frontend audio, use the core's `Audio` renderer described in
[API](API.md#audio).

Render captured buzzer drive from a complete trace:

```sh
uv run tools/verify_retail.py --quick --menu-trace --out out/audio-run
uv run tools/render_audio.py --trace out/audio-run/menu-events.txt \
  --report out/audio-run/menu/report.json --out out/audio-run/menu.wav
```

The WAV tool integrates differential drive over sample intervals. Acoustic filtering and
loudness calibration belong in a frontend. The tool rejects missing or truncated traces
and mismatched event counts.

`tools/preview.py` creates a standalone HTML viewer that displays captured PGM frames.
