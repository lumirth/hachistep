# Timing/hardware revision — engineering record

Baseline: `69f952d87bae3fd79e58c4ae4f72dfe69667af47`.
All implementation work takes place in the sandbox and builds on that Git tree.

## Register bus contract

Transcribed 95 register entries from REJ09B0152-0300 §20.1 pp.372–375 into
`conformance/spec/register_access.tsv`, independently of production routing.
SSU and the SCI core registers are three-state accesses; SPCR, IrCR, RTC,
Timer B1, Timer W, ADC, GPIO, AEC, and comparator accesses are two-state.
The CPU adapter still splits a logical word on an eight-bit register bus.
Both the table and timed guest reads are tested. Corrected missing PUCR1/PUCR3
writes, previously swallowed by an overlong source comment.

## Edge obligations

CPU pending work, SSU loading/half-edges, and ADC aperture/completion now retain
clock-source/divider edge obligations. An epoch-tagged timestamp is only a
projection cache. Gating holds unconsumed edges; resume rejoins shared divider
phase. Tests include a rate change during ADC conversion, SSU gating mid-byte,
and retained sample/result separation. This does not claim source-switch phase
characterization or convert SCI/oscillator-stabilization timing yet.

## Comparator implementation

Source: REJ09B0152-0300 §18, §21.2.5 and register/reset tables. Both comparators
have explicit result, baseline latch, read-armed enable, interrupt flag and
read-before-clear qualification. A same-time CDR read masks a newly generated
comparison interrupt without losing an older flag. This is local read-strobe
resolution, not CPU rollback. Comparator requests route to vector 36 and remain
operational in watch/standby when the module is enabled. Module standby while
CME remains set is rejected under §18.5's required software sequence.

There is an actual source inconsistency: CRS prose p.363 says VIL for non-
hysteresis, but Table 18.2 and Fig.18.2 specify VIH. The implemented VIH rule is
explicitly sourced to the latter, not called a new measurement. The 15 µs
conversion delay is the documented maximum used as a reproducible witness;
physical delay, offset and short-pulse response are not certified.

A complete guest program configures the comparator, arms its latch, sleeps,
wakes through vector 36 after an analog pin change, writes a RAM result, clears
and rearms the comparator, and executes RTE. The complete run equals 1,000
short partitions, including restoration during the pending analog transition.

Analog pin stimuli name real nodes. They do not invent a comparator wire or
convert step counts into register values. The default unconnected-node voltage,
ADC transfer, and CMOS threshold witnesses remain visible in STATUS.md.
