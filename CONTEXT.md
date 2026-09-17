# HachiStep

HachiStep is a Pokéwalker emulator core for running retail and custom firmware.

## Language

**Retail firmware**:
Firmware released for the physical Pokéwalker.

**Custom firmware**:
Firmware written or modified to run on the physical Pokéwalker, beyond its
retail firmware.

**pw**:
The matching Pokéwalker firmware decompilation, used as a reference for what the
retail firmware actually does.

**Hardware fidelity**:
Agreement between emulated and physical Pokéwalker behavior under matching
physical conditions, including timing and the consequences of power transitions.

**EEPROM save**:
The EEPROM contents used by Pokéwalker firmware for persistent progress.
_Avoid_: Save state

**Save state**:
A capture of the running Pokéwalker sufficient to resume from the captured
instant, including its memory, device state, and operations in progress.

**Snapshot**:
The in-memory representation of a save state.

**Exact restoration**:
Resuming a captured machine with the same subsequent hardware observations and
timing when given the same subsequent inputs.

**Best-effort restoration**:
Resuming a usable session without guaranteeing identical subsequent hardware
observations and timing.

**Single execution path**:
The sole production execution mechanism used to model the Pokéwalker for every
supported firmware image, with the same hardware fidelity in default operation.
_Avoid_: Fast mode, accurate mode
