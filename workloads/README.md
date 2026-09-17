# Retail workloads

`menu.csv` and `walking.csv` provide physical stimuli for the user's unmodified
private firmware and EEPROM. `retail.json` records reviewed software regression
expectations. These files exercise realistic integrated execution; their
emulator-observed hashes are not hardware truth.

Run them with `python3 tools/verify_retail.py --out out/retail-1`. Source inputs
are never changed. A justified hardware correction can change an expectation;
explain the cause and verify the mechanism before updating the baseline.

Independent guest diagnostics and their expectations live in
[hachiware](https://github.com/lumirth/hachiware).
