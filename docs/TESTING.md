# Behavioral validation

Use independent guest programs to check hardware behavior and complete firmware sessions
to check practical execution. Choose checks for the affected behavior and reuse completed
results until another change or unresolved failure warrants a run.

## Hardware diagnostics

Clone [Hachiware](https://github.com/lumirth/hachiware) alongside this checkout, then run:

```sh
uv run tools/check.py --out out/check-1
uv run tools/check.py --case 'ssu-*' --out out/ssu-1
```

The command checks formatting and Clippy, builds the release CLI, and executes the
selected Hachiware guests. Pass `--runner PATH` for an existing binary and
`--hachiware PATH` for another suite location. Results and failed observations are under
the output directory. Each destination must be new.

Hachiware owns expected hardware observations, with sources and assumptions beside each
case. When adding a case or workload, run its control audit as described in the
[benchmark guide](https://github.com/lumirth/hachiware/blob/main/docs/benchmark.md).

## Firmware sessions

Supply your firmware and EEPROM images:

```sh
uv run tools/verify_retail.py --firmware firmware.bin --eeprom eeprom.bin \
  --quick --out out/retail-1
```

`--list` describes the sessions without loading images. `--quick` runs home, menu, and
settings; repeat `--case NAME` to choose others. The longer sessions exercise motion,
idle operation, interrupted saves, connection timeout, and a day rollover. Capture
points compare the complete event history, exports, and resumed state against a
continuous session. `--chunk-us 1000` additionally compares repeated short host calls.

The optional `--play` and `--link` sessions use the matching retail firmware and an
EEPROM containing a walking Pokemon. They complete Dowsing and a capture, or a peer
exchange between two machines. Rewards and encounter records must reach EEPROM.
They use in-memory copies of the supplied save. Their firmware layouts follow
[`pw` at 6dc7bc0](https://github.com/lumirth/pw/tree/6dc7bc09950078fa3fe0dffa4dae34e9549a99da).

The [replay example](../crates/hs-core/examples/replay.rs) demonstrates frame delivery,
streaming audio, and snapshot restoration through the embedding API.

## Compare and measure

For Hachiware's per-workload speed and correctness comparison, including PocketWalker,
use its [benchmark guide](https://github.com/lumirth/hachiware/blob/main/docs/benchmark.md).

For two builds running the same private firmware:

```sh
uv run tools/bench.py --left /path/to/baseline --right /path/to/candidate \
  --firmware firmware.bin --eeprom eeprom.bin --input workloads/menu.csv \
  --milliseconds 6500 --repeats 4 --out out/paired
```

The tool first compares complete product histories, saved state, and exports, then times
untraced runs in alternating order. It accepts native executables and WASI modules.
Record the host, call horizon, samples, and spread with speed results.

Reports preserve commands and source, input, and executable identities. Keep generated
outputs under ignored `out/`. A changed source or executable invalidates that run.
Hardware agreement requires an independent expectation; agreement between two emulator
runs establishes reproducibility. Report remaining hardware or frontend gaps with the
results in the PR or issue.
