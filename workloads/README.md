# Firmware sessions

`retail.json` selects durations, physical input CSVs, and capture points for
`tools/verify_retail.py`. Run `--list` to see the selection.

The settings timeline changes Volume and confirms it. The two power-cut timelines
interrupt the primary or backup save. The firmware's mirrored-write and recovery rules
come from [`pw_local_settings.c`](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_local_settings.c)
and [`pw_eeprom_m95512_io.c`](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c).

Capture points fall during saves, a connection attempt, or the RTC update before
midnight. The verifier compares restored execution with the continuous session.
The manifest contains scenarios, without frozen emulator-output expectations.
