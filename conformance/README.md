# Pokéwalker diagnostic corpus — schema 2

This directory contains core-independent software fixtures and documented
expectations, not physical captures or complete hardware certification. The
builder imports no production Rust, decoder, or previous emulator. It emits the
actual instruction bytes that the same production CPU executes.

## Build and run

```sh
python3 conformance/build.py out/fixtures
python3 conformance/run.py --runner target/release/hachistep \
  --fixtures out/fixtures --report out/results.json
```

The generator requires a NEW output directory. It writes fifteen 49,152-byte
ROMs, a blank 65,536-byte EEPROM, applicable input CSVs, and `manifest.json`.
Its small encoder supports only the diagnostic forms it needs. There is no
special completion opcode or host-side mutation of guest result bytes.

| Cases | Expected observations |
|---|---|
| Register aliases, arithmetic flags, calls/returns, RAM execution | Explicit register/CCR/memory results through the production CPU. |
| Serial EEPROM page wrap | Guest SSU/GPIO commands buffer and commit a page-wrapped write. |
| Infrared transmit and receive | Pulse count and received byte for controlled nominal signals. |
| Three aliased predecrement stores | Byte, word, and long source data reflect the address update. |
| Timer W capture | A GPIO transition captures a stopped counter and enters vector 35. |
| Comparator | A read-armed analog crossing wakes the guest through vector 36. |
| AEC external overflow | External pulses update the counter and enter vector 32. |
| AEC gate | A selected gate edge enters vector 18, separately from overflow. |
| NMI | A masked sleeping CPU admits vector 7 and runs its ordinary guest ISR. |

## Fail-closed adapter contract

The manifest records the target, each ROM/input hash, EEPROM size/hash,
duration, a small expected-result map, and the basis of each expectation. The
adapter checks all input identities before execution; refuses empty or misspelled
established expectations; rejects out-of-range memory windows and paths escaping
the selected corpus; and checks every accepted scalar field, including interrupt
counts. A schema-1 manifest is not silently treated as schema 2.

The runner must export RAM, EEPROM and a report. RAM begins at 0xf780; EEPROM
begins at zero. Expected memory keys are four-digit hex addresses, with expected
bytes in ascending address order. Native word/counter behavior is tested through
actual guest accesses rather than directly setting a Rust field.

The exact requested endpoint is `floor(milliseconds * 2^64 / 1000)`. The adapter
compares that raw 64.64 value, not an incorrectly rounded microsecond display.
An eight-millisecond endpoint can display as 7,999 microseconds after truncating;
that is not an extra/missing emulated microsecond.

Outcomes are `pass`, `fail`, `unknown`, `not_applicable`, and `runner_error`.
Unknown/inapplicable cases are not green: exit code 2 distinguishes them from a
complete pass. Failed expectations/runner errors exit 1. A hardware-measured
expectation must identify an observation; none of the shipped cases claims one.

Another emulator can provide an adapter without adopting HachiStep's internal
state. The directory is separable but is not already a separate Git repository.

## Complementary tests

`spec/register_access.tsv` independently transcribes 95 physical register
width/access-state entries from the manufacturer tables. Rust boundary tests
consume it; it is not generated from the bus decoder. Integration tests elsewhere
sweep Timer W/AEC/comparator/clock/NMI transitions and snapshots.

`regressions/private-retail.json` is explicitly a **software-observed regression
baseline**, not independent hardware conformance. It contains only identities and
expected hashes/scalars, no private firmware bytes or artwork. Reviewed hardware
corrections may legitimately change it. The verifier never regenerates it.

`scenarios/menu.csv` and `walking.csv` supply integration stimuli to the user's
private retail images. Motion supplies physical acceleration only. The observed
107-step result depends on the current, still-provisional sensor model.

Keep focused cases small; expand interaction grids where mechanisms meet. Do not
inflate test counts by storing each generated operand combination as a new file.
