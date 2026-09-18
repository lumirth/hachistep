# Exact clock inversion

The retail idle profile in `out/profile-persistence/sample.txt` identified
`Clock::edges_before` and wide division among the largest sampled costs.
The core can remove that division without a clock cache or an alternative
executor.

A frequency N/D starts with period numerator `D * 2^64`. Integer clock
dividers preserve this form. With elapsed fixed-point time `delta`, retained
fraction `f`, and period numerator `p`, inversion previously computed
`floor((delta*N - 1 - f) / p)`. Write `p = d * 2^64`; this is exactly
`floor(((delta*N - 1 - f) >> 64) / d)`. Subtract before shifting: otherwise
an edge exactly at the exclusive horizon can be counted incorrectly.

The implementation now uses that identity with ordinary portable integer
operations. It adds no state, cache, target-specific code or guest shortcut.
The existing checked search remains for horizons whose scaled intermediate
overflows. Native validation verifies the period invariant; divided-clock
construction checks the complete rational numerator, including its remainder.
Boundary tests cover retained fractional phase, full-width periods and huge
horizons.

Benchmark preflight now compares canonical native `state.bin` as well as
all exported observations and complete product event histories. Timed runs
disable tracing and use paired ABBA order. The baseline is core `09ee905`,
binary SHA-256 `7fd6ea5ee5bd76cebb4e7a866c4a125e953438fdb62bb63bcc0e21391b53b063`.
Host: Apple M1, 16 GiB RAM, macOS 27.0 (26A428), Rust 1.98.1. Both release
binaries are 1,338,032 bytes; the change adds no runtime storage. The baseline
idle profile reported a 1,664 KiB peak physical footprint.

Median simulation-loop times (export and process startup are outside this
timer):

| Workload | Baseline | Exact reduction | Less time | Samples per binary |
| --- | ---: | ---: | ---: | ---: |

| Retail home, 10 s | 1.685795 s | 1.534237 s | 9.0% | 6 |
| Retail idle, 120 s | 11.763565 s | 10.835907 s | 7.9% | 4 |
| Custom EB5 eraser, first 90 ms | 0.016367 s | 0.014862 s | 9.2% | 12 |

Receipts are `out/clock-bench-home`, `out/clock-bench-idle` and
`out/clock-bench-flash`. Every preflight passed native-state, export and complete
history equality. The custom guest runs the documented erase/verify algorithm
from RAM: at the 90 ms horizon it has retired 110,133 instructions and ended
eight erase pulses, with the update still in progress. The short custom sample
has more timing spread than the retail runs; raw samples are retained.

These measurements apply to these workloads and host; they do not establish a
ranking against other emulators.

Full validation: `out/clock-reduction-check` passes format, debug/release/trace
tests, Clippy, host tooling and all 123 independent guests.
`out/clock-reduction-retail` preserves all four reviewed retail workloads and
partition/native replay; the private input files remain unchanged.
