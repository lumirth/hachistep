# Testing and evidence

## Different claims have different tests

The standard suite needs no private images. It runs ordinary Rust tests,
documentation tests, an all-features/trace build, Clippy, host-tool checks, and
fifteen independently encoded guest fixtures. Run:

```sh
python3 tools/check.py --out out/check
python3 tools/mutation_check.py --out out/mutations
python3 tools/verify_retail.py --out out/retail --menu-trace
```

All destinations must be new. No compiler/dependency is installed and no network
CI job is created. See `evidence/next-revision` for actual results and versions.
There are no new physical hardware captures.

The private Rust test compares the **complete typed machine state and complete
product-event vectors** across randomized run partitions, then a further interval
after snapshot restoration. The peripheral integration tests add clock/gate,
analog-input, capture, AEC and NMI partition/snapshot cases. These establish
representation/replay consistency, not agreement with silicon.

## Independent target expectations

| Location | Scope |
|---|---|
| `hachiware/spec/register_access.tsv`, `tests/register_access.rs` | Independent register reference data in hachiware; local guest-access boundary regression. |
| `tests/cpu_regressions.rs` | 120 aliased predecrement combinations, partial long stores, RTE/LDC admission, 192 displacement-24 cases, EEPMOV/NMI and stable issued actions. |
| `tests/clock_obligations.rs` | CPU/SSU/ADC source-edge waits, downstream gating, source changes, ordinary GPIO pull writes. |
| `tests/comparators.rs` | Dual-channel hysteresis/reference/arming/clear behavior, guest wake, live gating, ignored external ladder selections and channel vectors 21/22, peek and pin-routing tests. |
| `tests/timer_w_modes.rs` | Buffers, capture, external clock, local conflicts, PWM boundary, stabilization gating and guest vector 35. |
| `tests/aec.rs` | Counter/PWM/gate recurrence, 8/16-bit behavior, separate flags/requests, guest vectors 18/32, shared phases and replay. |
| `tests/nmi.rs` | Dedicated edge latch, masked wake, held-input behavior, standby replay, reset straps and failed power-on nonmutation. |
| Separate `hachiware` repository | Guest images, independent literal expectations, hashed manifests, and runner. HachiStep provides `tools/hachiware_adapter.py`. |

Other existing tests cover arithmetic, image/CLI safety, memory and serial
mechanisms, typed snapshots, and ordinary-run zero allocation. First-word
nonpanic enumeration is retained, but is **not** called complete opcode
certification. Many subcycle timings and physical parameters remain provisional;
STATUS records them rather than hiding them behind passing tests.

## Mutation checks

The mutation tool copies only sources and expectation data into an automatically
cleaned temporary directory. It runs the unmodified control, changes exactly one
identified site, and requires a compiled test to fail with the expected assertion.
A compiler error, missing test, zero tests, or unapplied mutation is not success.
The source repository and user's machine are never modified.

The three shipped mutations reintroduce a wrong RTC access duration, capture an
aliased predecrement source too early, and suppress EEPMOV.W NMI acceptance.
These tests demonstrate sensitivity to those bugs. They do not establish that
all possible bugs are detected, or that the expectations came from physical runs.

## Private firmware: regression versus smoke

`verify_retail.py` now checks the exact firmware/EEPROM identity and scenario,
semantic report fields and all six exported images against
`workloads/retail.json`. This includes the frame bytes,
rather than merely recording a screenshot hash while asserting only completion.
The supplied home/menu/walking frames were visually inspected to establish this
software regression baseline. A model-correcting change may legitimately require
a separately reviewed expectation change; never auto-rebaseline from a candidate.

Use `--smoke-only` with different private inputs to test execution only. Its output
explicitly says that no behavioral regression comparison was made. It is a host
verification choice, not an emulator accuracy mode; the execution engine is the
same. `--quick` omits the longer walking/idle workloads.

The 107-step result is an observation of a synthetic trajectory under the current
sensor model, not an independently measured physical pedometer expectation.

## Compare histories before measuring speed

```sh
python3 tools/compare_runs.py out/left out/right \
  --left-trace out/left-events.txt --right-trace out/right-events.txt
python3 tools/bench.py --left /path/to/baseline --right /path/to/candidate \
  --input workloads/menu.csv --milliseconds 6500 \
  --repeats 4 --out out/paired
```

The comparator checks semantic report fields, canonical native state, exported bytes and, when
requested, every product-event record. It identifies the first difference and
rejects truncated, missing, mixed bus/product, or report-length-mismatched
histories. It does not equate endpoint equality with history equality. The
reviewed retail baselines record hardware observations, while equivalence runs
also compare the current native capture of causal internal state.

A paired benchmark first performs two **untimed** complete-product-history runs.
Measured runs do not trace or hash every event in the simulation path. They use
ABBA/BAAB ordering and preserve raw samples. An accuracy correction that changes
behavior must establish a new justified baseline first, not masquerade as a
performance-only improvement. One-binary runs are measurements without a
cross-build equivalence claim.

CLI simulation-loop time and externally measured process/load/export time are
reported separately. Neither is energy or physical accuracy. Confidence and
cross-host generality must not be inferred from a small single-host sample. The
0.2 measurement in REVISION-0.2 compares only the immediately preceding complete
new-hardware build with its inactive-AEC optimization, not the original starter
or another emulator.

## Host-tool tests

Tests cover import/no-clobber/path safety, PGM pixel preservation, aperture audio,
truncated histories, result-manifest input hashes/schema, actual IRQ assertions,
software regression frame/identity/duration checks and missing expectations.
These tests also deliberately change observed records while keeping final state
unchanged, so an endpoint-only comparison cannot accidentally claim history
coverage.
