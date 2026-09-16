# Pokéwalker diagnostic starter corpus

These are original, core-independent software fixtures. They are **not** a claim
of complete H8/MCU coverage or a set of physical captures. No production Rust is
imported while generating them. Their expected results are explicit byte values,
register values and event counts derived for the controlled cases.

## Build and run

```
python3 conformance/build.py out/fixtures
python3 conformance/run.py --runner target/release/hachistep \
  --fixtures out/fixtures --report out/results.json
```

The generator takes a positional NEW output directory. It emits 49,152-byte ROM
images, a blank 65,536-byte EEPROM, optional physical-input CSV and `manifest.json`.
It uses a tiny encoder for just the required forms; it is not a general assembler.
The programs begin at 0x0100, establish a stack, write result bytes when needed,
and finish in an ordinary branch loop. There is no magic emulator completion
opcode and no host-side mutation of the result registers.

## Initial cases

| Case | Observation |
|---|---|
| `register-aliases` | Byte/high-word writes preserve the appropriate ER0 portions; memory result `ccddaabb`. |
| `add-byte-flags` | 0x80 + 0x80 gives byte zero and the saved CCR value 0x87 from this initial state. |
| `call-return-stack` | Subroutine result 0x2b and expected saved return address at the stack location. |
| `execute-from-ram` | Guest stores a short instruction stream into RAM and executes it through JMP. |
| `serial-eeprom-page-wrap` | Guest configures SSU/GPIO, executes WREN and WRITE, crosses a 128-byte page boundary and waits through the modeled commit. |
| `infrared-transmit` | Guest transmitter sends 0xa5; expected optical pulse-transition count is ten. |
| `infrared-receive` | A nominal SIR pulse CSV is consumed by the modeled receiver and stored as 0xa5. |

The output format includes a ROM SHA-256, duration, optional input filename and
literal expected observations. The current adapter runs a CLI into a temporary
new directory, validates the image hash, checks byte ranges from `ram.bin` and
`eeprom.bin`, and checks appropriate values from `report.json`. Another emulator
can implement its own adapter to those observations; it need not adopt HachiStep
internals or filesystem layout.

Address dictionaries use four-digit hexadecimal target addresses; expected bytes
are a hex string in ascending address order. RAM output begins at 0xf780; EEPROM
output begins at zero. Expected result fields are intentionally small, not a
serialized copy of a second machine.

## Growth policy

Keep one question per focused fixture, then add interaction grids where needed.
Record hardware revision, setup, direct versus inferred observation, and unresolved
expectations before introducing actual capture data. A fixture failure, unknown
hardware expectation, inapplicable revision and runner failure must not eventually
be collapsed into the same percentage. The current seven cases have known
software expectations; no capture database or empty research framework is shipped.

The scenarios directory is separate: menu.csv and walking.csv are integration
stimuli for the supplied private retail images. They are not standalone hardware
conformance programs. walking.csv supplies physical acceleration only; its observed
step count is firmware output and depends on the current sensor model.
