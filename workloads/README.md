# Retail workloads

`retail.json` defines the scenarios, durations, input files, capture points and reviewed
software regression expectations. CSV timelines supply physical stimuli to the user's
unmodified private firmware and EEPROM. Their recorded hashes protect observed behavior.

Run them with `uv run tools/verify_retail.py --out out/retail-1`. Use `--list` to
inspect the scenarios and repeat `--case NAME` to select them. A justified hardware
correction can change an expectation;
explain the cause and verify the mechanism before updating the baseline.

The corpus covers home, menu navigation, motion processing, idle, a settings save,
two interrupted saves, and an infrared attempt without a peer. The settings sequence
opens Volume, changes it once and confirms. The firmware writes its 24-byte save and
checksum to each EEPROM mirror. Cutting power during the primary checksum preserves the
old valid backup; cutting during the backup payload preserves the new valid primary.
Both cold starts repair the other copy. These expectations follow the matching
[`pw` settings](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_local_settings.c)
and [mirror routines](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c).
The absent-peer attempt ends at the firmware's "No Trainer found" screen.

Scenarios with `checkpoint_ms` also capture during execution and restore from disk.
Verification compares the complete joined event history and final native state with an
uninterrupted run. The persistence cases capture inside a page or checksum write.
The communication case captures during the attempt, before its timeout.

Independent guest diagnostics and their expectations live in
[hachiware](https://github.com/lumirth/hachiware).
