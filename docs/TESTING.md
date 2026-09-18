# Testing and evidence

## Different claims have different tests

The standard suite needs no private images. It runs ordinary Rust tests, documentation
tests, an all-features/trace build, Clippy, host-tool checks, and the independently
encoded guest fixtures in hachiware. Run:

```sh
python3 tools/check.py --out out/check
python3 tools/mutation_check.py --out out/mutations
python3 tools/verify_retail.py --out out/retail --menu-trace
```

All destinations must be new. Check outputs record the source revision, commands,
toolchain, and results. Historical receipts in `evidence/` describe their own revisions;
they do not establish the current checkout's coverage.

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
report fields and all six exported images, including frame bytes, against
`workloads/retail.json`. The reviewed observations form a software regression baseline. A
model-correcting change may legitimately require a separately reviewed expectation
change; never auto-rebaseline from a candidate.

Use `--smoke-only` with different private inputs to test execution only. Its output
explicitly says that no behavioral regression comparison was made. It is a host
verification choice, not an emulator accuracy mode; the execution engine is the same.
`--quick` omits the longer walking/idle workloads.

A step count produced from a synthetic trajectory records the emulator's pedometer
response to that input.

## Compare histories before measuring speed

```sh
python3 tools/compare_runs.py out/left out/right \
  --left-trace out/left-events.txt --right-trace out/right-events.txt
python3 tools/bench.py --left /path/to/baseline --right /path/to/candidate \
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
separately. Report the host and sample spread with each result.

## Host-tool tests

Tests cover import/no-clobber/path safety, PGM pixel preservation, aperture audio,
truncated histories, result-manifest input hashes/schema, actual IRQ assertions,
software regression frame/identity/duration checks and missing expectations. These tests
also deliberately change observed records while keeping final state unchanged, so an
endpoint-only comparison cannot accidentally claim history coverage.
