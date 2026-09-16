# Testing and evidence

## What was actually run

The release checks comprise 47 ordinary Rust tests, one compiled documentation
example, and one separately opted-in private retail test. The standard Rust
corpus was also repeated with the `trace` feature enabled. Five Python host-tool
tests and seven independent guest diagnostic fixtures pass. Formatting, Clippy
with warnings denied, offline debug/release compilation and an extracted-package
rebuild are recorded in `evidence/` when present.

The private test runs the supplied firmware and buttons to five seconds in one
call and in randomized short calls. It compares the **entire product-event
vectors and the complete machine states**, not just final screenshots. It then
restores a typed snapshot and compares a further second of complete state and
events. No hardware fixture was connected; these are software regression results.

A separate real-firmware script runs home, menu, synthetic motion and long idle
workloads. It checks completion and integration activity, records all endpoint
hashes/counters, and verifies the original input hashes remain unchanged. The
107-step screenshot is an observed result of the supplied trajectory and model,
not a claim about pedometer accuracy on real motion.

## Rust test locations

| Location | Purpose |
|---|---|
| `cpu/alu.rs` | Exhaustive byte arithmetic input cases and chained/preserved flags. |
| `cpu/decode.rs` | Known forms/extensions and classification of all first words without panic. |
| `cpu/mod.rs` | Aliases, reset prologue and exception write order. |
| Owner-local tests | Serial/parser, timing, register and sampling mechanisms. |
| `tests/kernel.rs` | Whole-machine partition/snapshot, horizons, invalid timelines, RAM execution, faults, real serial programming, lifecycle and persistence import. |
| `tests/allocation.rs` | Allocation-free ordinary run with an allocation-free sink. Test-only allocator instrumentation uses `unsafe`; production core forbids it. |
| `tests/retail.rs` | Opt-in complete event/state replay against private images. |
| `hs-cli/tests/safety.rs` | Invalid input handling, no-clobber output, persistence images and run metadata. |
| Crate doctest / `examples/replay.rs` | Compiled API usage and host integration. |

This is a starter corpus, not a full H8 conformance matrix. Enumeration without a
panic does not prove that every reserved/valid encoding is correctly classified.
A local expected value expressed from the same algorithm can show consistency
without providing independent physical evidence. See STATUS for unclosed cases.

## Core-independent diagnostic fixtures

`conformance/build.py` emits small original H8 programs with literal expected
outcomes. It does not import Rust, `hs-core`, its decoder, or a previous emulator.
`conformance/run.py` is a thin CLI adapter that checks RAM/EEPROM/register/output
observations. These are software-reasoned expectations, not recorded hardware.
The format and cases are documented in `conformance/README.md`.

```sh
python3 conformance/build.py out/fixtures
python3 conformance/run.py --runner target/release/hachistep \
  --fixtures out/fixtures --report out/fixture-results.json
```

The fixtures can be split into a separate repository as they grow. There is no
second production CPU implementation or universal test DSL to maintain.

## Host tool tests

The Python unit tests exercise archive extraction without path traversal,
no-clobber destinations, rejection before writing invalid input, binary PGM pixel
preservation, aperture-integrated audio and trace-truncation rejection. The
end-to-end release process additionally runs these tools on the real files.

## Measurements

```sh
python3 tools/bench.py --milliseconds 10000 --repeats 3 --out out/bench
python3 tools/bench.py --left /path/to/baseline --right /path/to/candidate \
  --input conformance/scenarios/menu.csv --milliseconds 6500 \
  --repeats 5 --out out/paired
```

With two binaries, the tool varies ABBA/BAAB paired ordering using a fixed seed.
It preserves each raw report and reports median/min/max times. It does not
invent confidence intervals from a handful of samples. Endpoint and event-count
mismatches reject the comparison; they are not ignored to obtain a speed number.

The CLI's internal wall interval measures the simulation loop, with any selected
trace work. The external timer additionally includes process launch, image load,
construction and export. Both are recorded. No output-based time is an energy
measurement. No result is a competitive fastest-emulator claim.

The initial three default-build 10-second runs here had a median simulation-loop
wall time of approximately 1.05 seconds on the sandbox's Linux x86-64 host. The
raw samples, binary hash and exact inputs are in `evidence/benchmark.json`. This
is not a controlled cross-emulator benchmark or a prediction for an Apple Watch.

## Adding accuracy tests

Start with the first incorrect observable access or event. Preserve the relevant
clock/interrupt/history context when reducing it. Put mathematical optimization
invariants in local tests; put independent target behavior in diagnostic fixtures
or device signal cases. Mark measured, documented and provisional expectations
separately. Do not regenerate golden observations from the candidate implementation
and label the result independent certification.
