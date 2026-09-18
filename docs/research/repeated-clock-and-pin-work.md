# Repeated clock and pin work

The retail profile in `out/profile-i2c/sample.txt` placed clock inversion and
GPIO resolution among the largest sampled costs. Two operations were repeating
work whose result was already settled:

- Stopping a source advances its clock to the last emitted edge. A stopped
  derived source is likewise constructed with its count settled at the held
  instant. Its tick count can therefore return that ordinal directly. Native
  validation now checks this invariant, and a boundary test stops before, on
  and after an edge, waits through `Time::MAX`, and restarts without losing or
  manufacturing ticks.
- Board resolution computed every GPIO pad twice, even when neither serial
  slave changed its drive. Between those two resolutions, only the sensor and
  EEPROM drivers can change a resolver input. Comparing their two drive values
  lets the second call be omitted when all electrical inputs are identical.
  Changed slave outputs still settle at the same instant, before subsequent
  pin consumers and native capture.

Neither change adds a cache, retained field, hardware timing rule or execution
backend. They remove redundant calculation from the existing mechanisms.

## Measurements

Baseline: `1eedaf574b4e56bea443d9cbc07e2322f87ddab5`, binary SHA-256
`ff97c79dd9094770b395f71ee2d8d02e5ba680207cbe73dc9dfa6027cc7adda5`.
Candidate binary SHA-256:
`4680b7be0543bac49fa108d098c161f657f111ad2f16c09300acb95c13746fd1`.
Host: Apple M1, 16 GiB RAM, macOS 27.0 (26A428), Rust 1.98.1.
The binaries occupy 1,393,184 and 1,393,248 bytes. No runtime storage was added;
native files are byte-for-byte identical for each paired preflight. The sampled
baseline process had a 1,696 KiB peak physical footprint.

Every comparison first checks canonical native state, exported observations
and complete product-event histories. Timed runs omit tracing and use paired
ABBA/BAAB order. No builds or other emulator measurements run concurrently.

| Workload | Baseline median | Candidate median | Less time | Samples per binary |
| --- | ---: | ---: | ---: | ---: |
| Retail home, 10 s | 1.546176 s | 1.462324 s | 5.4% | 6 |
| Retail idle, 120 s | 10.849524 s | 10.166216 s | 6.3% | 4 |
| Custom EB5 eraser, first 90 ms | 0.014998 s | 0.014956 s | 0.3% | 12 |

The custom guest runs the actual RAM-resident erase/verify algorithm, with the
update still in progress at capture. Its 0.3% difference is smaller than the
observed spread and does not establish a speed improvement. The two retail
workloads show separated timing ranges in this run. Results describe this host
and these workloads, not an emulator ranking or a guarantee for every firmware.

Raw receipts are `out/settled-work-home`, `out/settled-work-idle` and
`out/settled-work-flash`. The earlier `out/stopped-clock-home` run isolated the
ordinal change before the GPIO change; the table above measures the final code.

Full validation passes in `out/settled-work-check`: formatting, debug/release/
trace tests, Clippy, host tooling and all 132 independent guests. All four
reviewed retail workloads and native replay remain unchanged in
`out/settled-work-retail`, and private input identities are preserved. The five
clock owner tests also pass on Rust 1.95.0 (`out/settled-work-msrv.log`).
