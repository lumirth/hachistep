# Testing and evidence

## Different claims have different tests

The standard suite needs no private images. It runs ordinary Rust tests, documentation
tests, an all-features/trace build, Clippy, host-tool checks, and the independently
encoded guest fixtures in hachiware. Run:

```sh
uv run tools/check.py --out out/check
uv run tools/mutation_check.py --out out/mutations
uv run tools/verify_retail.py --out out/retail --menu-trace
```

All destinations must be new. Reports record the source revision, commands, toolchain
and results under ignored `out/`. Keep a reviewed expectation or supporting hardware
capture with the test that uses it. Individual run reports stay local.

Check, retail, benchmark and mutation summaries fingerprint the checkout before and after
execution. `tree_sha256` covers tracked and nonignored untracked file contents, names,
executable bits and symlink targets; ignored outputs and private inputs are excluded.
A changed fingerprint fails the run after saving its summary. An unavailable fingerprint
leaves `source_unchanged` null. Keep outputs in an ignored directory so they do not change
the fingerprint themselves.

These fingerprints identify content but do not preserve it or detect edits reverted
between the two observations. A supplied executable's hash identifies that file; it does
not establish which source built it. Preserve the corresponding source and build record
when retaining a comparison.

Choose additional checks for the affected contract. Native/Wasm comparisons belong with
changes that could behave differently by target, such as arithmetic representation,
encoded layouts, target support or host integration, and with release validation. Reuse
completed checks until a change, failure or unresolved concern warrants another run.

The private replay test compares the complete typed machine state and complete
product-event vectors across randomized run partitions, then a further interval after
snapshot restoration. The peripheral integration tests add clock/gate, analog-input,
capture, AEC and NMI partition/snapshot cases. These establish representation/replay
consistency, not agreement with silicon.

## Independent target expectations

[hachiware](https://github.com/lumirth/hachiware) owns hardware diagnostics,
signal fixtures, and their expected observations. HachiStep's
`tools/hachiware_adapter.py` runs them through production execution and exports
observations. Expected results come from hardware documentation, measurements, firmware
evidence, or justified inference; the emulator must not generate its own hardware
oracle.

List or rerun selected diagnostics using the fixtures produced by `check.py`:

```sh
uv run ../hachiware/run.py --fixtures out/check/fixtures --list --case 'adc-*'
uv run ../hachiware/run.py --fixtures out/check/fixtures --case 'adc-*' \
  --adapter tools/hachiware_adapter.py --runner target/release/hachistep \
  --out out/adc-check
```

The adapter advertises available observations, inputs and configured conditions.
Hachiware requests only the observations each case checks. The adapter verifies the
requested endpoint using HachiStep's clock representation. A failed experiment retains
its observations and logs; `--keep-passed` also retains successful exports.

Local tests protect embedding contracts, timing and access regressions, partition/save
state behavior, resource guarantees, and host-tool behavior. Prefer observations that
survive a different internal implementation. Opcode classification without panics,
private-state equality, or a passing retail run answers a narrower question than
hardware conformance.

## Mutation checks

The mutation tool copies only sources and expectation data into an automatically cleaned
temporary directory. It runs the unmodified control, changes exactly one identified
site, and requires a compiled test to fail with the expected assertion. A compiler
error, missing test, zero tests, or unapplied mutation is not success. The tool leaves
repository sources intact and removes its temporary copy.

The three shipped mutations reintroduce a wrong RTC access duration, capture an aliased
predecrement source too early, and suppress EEPMOV.W NMI acceptance. These tests
demonstrate sensitivity to those bugs.

## Private firmware: regression versus smoke

`verify_retail.py` checks the exact firmware/EEPROM identity and scenario, semantic
report fields and all exported images, including frame bytes, against
`workloads/retail.json`. The reviewed observations form a software regression baseline. A
model-correcting change may legitimately require a separately reviewed expectation
change; never auto-rebaseline from a candidate.

Use `--smoke-only` with different private inputs to test execution only. Its output
explicitly says that no behavioral regression comparison was made. It is a host
verification choice, not an emulator accuracy mode; the execution engine is the same.
`--quick` selects home, menu and settings. Use `--list` to inspect the corpus or repeat
`--case NAME` for a focused run. The manifest owns scenario selection and durations.

Settings changes and interrupted saves exercise firmware persistence and mirror repair.
The infrared timeout exercises an attempt without a peer. Their capture points also
check restoration during ongoing operations: complete traces before and after loading
must concatenate to the continuous history, and final native state and exports must
match. Captured traces are stored as `events.txt` in each scenario's output directory.

The peer exchange test runs independent machines through the public optical interface
and checks the encounter records committed by both firmwares. Its input preparation and
simulated channel are described in [INPUTS](INPUTS.md#physical-csv). It runs alongside
the partition test for every retail verification, including `--quick` and `--case`.

The gameplay test continues the walking trajectory into Dowsing, spends earned Watts,
collects an item and checks its persistent record. A separate Poké Radar session uses a
host RAM edit to fund entry, then completes the search, battle and capture through
button inputs. The guest writes the captured Pokémon from its course data. Both sessions
check sound, display, complete event replay and exact restoration during play. Firmware
layouts and expected rewards come from the matching `pw` source; this test exercises the
public embedding API and does not add firmware knowledge to the core.

The day-rollover workload runs for an emulated day, including idle sleep, hourly saves,
diary rotation and midnight maintenance. Its capture falls inside the RTC busy interval
before midnight, so restoration must preserve the pending calendar update and all later
firmware writes. The reviewed results follow `pw`'s RTC and diary routines: elapsed
hours and days advance, save mirrors retain valid checksums, and midnight clears the ten
peer-history device IDs while preserving the received staging record and other fields.
Run it alone with `--case day-rollover`.

A step count produced from a synthetic trajectory records the emulator's pedometer
response to that input.

## Compare histories before measuring speed

```sh
uv run tools/compare_runs.py out/left out/right \
  --left-trace out/left-events.txt --right-trace out/right-events.txt
uv run tools/bench.py --left /path/to/baseline --right /path/to/candidate \
  --input workloads/menu.csv --milliseconds 6500 \
  --repeats 4 --out out/paired
```

The comparator checks semantic report fields, canonical native state, exported bytes
and, when requested, every product-event record. It identifies the first difference and
rejects truncated, missing, mixed bus/product, or report-length-mismatched histories. It
does not equate endpoint equality with history equality. The retail baselines record
observations from software execution. Equivalence runs also compare native captures of
causal internal state.

A paired benchmark first performs two untimed complete-product-history runs. Measured
runs exclude that tracing and hashing. They use ABBA/BAAB ordering and preserve raw
samples. An accuracy correction that changes behavior requires a new justified baseline
before performance comparison. A run with one binary measures its performance alone.

CLI simulation-loop time and externally measured process/load/export time are reported
separately. Use `--chunk-us` to measure frequent calls over the same firmware workload;
the default is 1000 microseconds. Record the call horizon, host and sample spread with
each result. The report retains the horizon, tool versions and binary hashes.
Both executable arguments also accept the `wasm32-wasip1` CLI module described in
[BUILD](BUILD.md#webassembly-checks). This permits the same event and state comparison
across native and Wasm execution.

## Host-tool tests

Tests cover output safety, PGM pixel preservation, aperture audio,
truncated histories, fixture input hashes, actual IRQ assertions, adapter completion,
software regression frame/identity/duration checks and missing expectations. These tests
also deliberately change observed records while keeping final state unchanged, so an
endpoint-only comparison cannot accidentally claim history coverage.
