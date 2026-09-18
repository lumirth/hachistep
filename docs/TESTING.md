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

All destinations must be new. Check outputs record the source revision, commands,
toolchain, and results under ignored `out/`. Keep a reviewed expectation or supporting
hardware capture with the test that uses it. Individual run reports stay local.

The private Rust test compares the complete typed machine state and complete
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

## Host-tool tests

Tests cover output safety, PGM pixel preservation, aperture audio,
truncated histories, fixture input hashes, actual IRQ assertions, adapter completion,
software regression frame/identity/duration checks and missing expectations. These tests
also deliberately change observed records while keeping final state unchanged, so an
endpoint-only comparison cannot accidentally claim history coverage.
