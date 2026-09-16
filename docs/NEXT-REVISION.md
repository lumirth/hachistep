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

## Timer W and stabilization

Implemented paired compare buffers and capture buffers, FTIO capture, and FTCI
external rising-edge counting through one counter recurrence. Every compare
uses the same old-register snapshot before buffer loads. Buffer registers can
still produce their own compare outputs (§10.7.12). Equal PWM period/duty
matches now retain the existing output instead of forcing initial polarity.
TCNT clear wins a simultaneous CPU write; CPU GR writes win capture/buffer
transfers without suppressing status. Reads at a capture boundary see the old
GR until the following reference edge. CTS=0 stops counting, not capture.
Module standby retains pending flags and prevents clearing them (§10.7.4).

The input pipeline models Figs.10.15/10.17 with three reference-edge stages.
Metastability, exact sub-state propagation, and short out-of-spec pulses have
not been characterized. Switching clock muxes may itself produce an increment
(§10.7.3); that mechanism remains open, not silently claimed complete.

Timer W now stops in watch/standby and oscillator stabilization, while supported
watch/external counting remains available in subactive/subsleep. Stabilization
is explicit control state: system-clocked SSU/ADC/SCI do not resume before its
completion. Timer output A is routed to P10; B/C/D use P82/83/84. A full guest
fixture drives a physical GPIO edge, captures a stopped counter, takes vector
35, writes the captured word to RAM, clears the flag and returns. Full firmware
partition/snapshot replay still passes.
