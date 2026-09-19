# Timer B1, Timer W and AEC

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

Timer B1 implements interval/reload counting, load/counter aliasing, source selection,
overflow requests and retention across the applicable power modes. Timer W implements
counting, compare/capture, buffering, output modes, external inputs and the documented
input pipelines and access conflicts. The clocks and resolved pins supply their edges.
Firmware's Timer W initialization provides an independent example of the watch-clock
configuration used for timing.

AEC supports independent/cascaded counters, external edge counting, PWM gates and
outputs, overflow requests and IRQAEC. External counting can continue with CPU clocks
stopped where specified. PWM period and low time use register value plus one; duty
greater than or equal to period forces the output low. Live changes traverse the
same gate and pin logic as clock-driven changes.

## Limits and open questions

Live source, mode and load changes retain progress under local rules where the manual
prescribes stopping first. Review same-time capture/compare/register accesses and
clock or pin changes against the manual's conflict diagrams. This is an interaction
review target, not a claim that these supported operations are absent. The additional
Timer B1 application note in the catalogue supplies a useful independent reload example.

The exact miscount after a prohibited cascade-enable sequence, synchronizer aperture
and some live PWM transitions remain inferred. Documented restrictions explain why
these sequences deserve review, but do not themselves specify the missing result.
The manufacturer's AEC PWM example gives an additional period/duty configuration to
compare with the current model.

## Sources and applicability

The H8/38606 target addition leaves these peripherals unchanged. The applicable baseline
is REJ09B0152-0300 rev. 3.00. Page references are printed manual pages; PDF pages are 34
higher. ([Target applicability][target], [manual][manual])

## Timer B1

`F0D0 TMB1` resets to `38`: bit 7 selects reload, bit 6 enables counting, bits 2–0
select `φ/8192, /2048, /256, /64, /16, /4, φW/1024, φW/256`. `F0D1` reads TCB1 and
writes TLB1. A stopped TLB1 write updates both load and counter, including interval
mode. Overflow sets IRR2.IRRTB1, and reload occurs on that overflow; the first traversal
is `256-count`, subsequent traversals are `256-load` in reload mode. ([Manual §§9.2–9.4,
pp. 146–150][timer])

The supported setting procedure explicitly stops the counter before changing mode,
source or load. Running changes are not documented as ignored or as CPU faults. The
selected inference is to accept the register write; a live TLB1 write loads both
latches, while mode/source changes retain the counter. Continue from the newly selected
shared prescaler phase. A mux-induced edge can be evaluated through the existing
source-level convention; do not automatically grant a new full period.

Both sources run in active/sleep. Watch-source counting also runs in watch,
subactive/subsleep, and their wake stabilization. All B1 counting halts in standby and
standby wake stabilization; counters are retained. System-source counting stops outside
active/sleep. Module standby retains state; reset clears mode/count/load. ([Manual table
9.1, p. 151; §20.3][timer-power])

## Timer W

Timer W provides compare outputs, PWM, capture and paired buffers. Its count sources
include system and watch divisions and external FTCI. The selected operating mode
controls whether those sources can advance the timer. The pin and access rules in
manual §§10.2–10.7 are as important as the counter values.
[Registers and operation](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=190).

The implementation uses one counter recurrence for internal clocks and synchronized
external edges. `counter_step` evaluates all comparisons against the pre-transition
register values, records flags and updates outputs, then transfers paired buffers.
This ordering also preserves a buffer register's own compare output. Stable spans
advance to the next compare or overflow without processing every unused count.

The conflict rules preserve these observations:

| Coincident events | Result |
| --- | --- |
| TCNT write and counter clear | Clear wins. A write wins over an ordinary increment. |
| Capture and general-register write | The CPU value remains; the capture flag still sets. |
| Capture and general-register read | The old value remains readable until one reference clock after capture. |
| Compare and general-register write | The output reflects the comparison; the CPU value remains afterward. |
| Buffered transfer and buffer write | The transfer uses the old buffer value; the buffer retains the CPU write. |

These rules come from [§10.7](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=215).

`input_pins` feeds capture and FTCI through a retained three-stage input pipeline.
Selecting a previously disconnected input establishes its baseline. Capture can still
set a flag while counting is stopped. The pipeline and delayed capture visibility use
the CPU reference clock, independently of the source selected for counting.
The implementation and its tests interpret figures 10.15 and 10.17 in this way.
[Input timing](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=209).

The model also retains the low-to-high edge caused by switching internal clock taps.
Module standby preserves pending status and prevents software from clearing it through
the stopped module. A simultaneous PWM period and duty match retains the output level.
The tests below exercise each of these distinctions.

Some boundaries still deserve targeted review. The manual warns of a possible one-count
misalignment on active-to-subactive transitions without specifying its complete phase
condition. The model follows the selected clock phase; the condition for that warning
is not separately established. Very short input pulses and selecting a mux while an
input is already active also depend on the chosen sampling and baseline rules. Check
those specific sequences before treating them as established physical behavior.

`pw` uses the watch-clock timer for infrared timeouts and Timer W PWM for the buzzer.
Its [IR initialization](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L158-L180)
and [buzzer setup](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_buzzer.c#L49-L65)
corroborate those uses; the conflict rules above have their own manual basis.

## AEC: active reconfiguration

Register writes and clock selection follow these rules:

| Access or configuration | Behavior |
| --- | --- |
| ECPWDR read | Return the selected zero bus value; it is write-only with undefined read value, not a CPU fault. |
| AEGSR bit 0, ECCR bit 0, ECCSR bit 5 | Store/read them: the manual explicitly calls these reserved bits readable/writable. They have no modeled function. |
| Edge selection `11` | Retain encoding; select neither edge as the local inference. |
| PWCK `111` | Retain encoding and PWM phase; disconnect its clock as the local inference. |
| Active period/duty write | Update the latch and retain the 16-bit PWM count and output phase. Compute the next comparator equality with wrapping distance, not subtraction that assumes count ≤ new threshold. |
| Active PWCK change | Keep PWM count/output; select the new divider's existing phase. |
| Live CUEH/CH2 changes | Preserve H/L counts except explicit CRCH/CRCL reset; update the cascade/independent clock gates. Apply any resulting clock edge through the same counter mechanism. |

The running period/duty/clock transitions above are inferences: software is told to stop
PWM first. Stable waveform facts are exact: low time is `(ECPWDR+1)` clocks, full period
`(ECPWCR+1)` clocks, and duty ≥ period forces IECPWM low. A live write causing that last
condition must also produce the corresponding gate transition; merely suppressing future
deadlines can leave the output high. ([Manual §§13.3–13.4.5, pp. 216–227][aec])

For 16-bit operation the manual specifically warns that changing CUEH after CRCH release
can miscount ECH; it also requires CUEH set before or together with CRCH release.
Consequently the cascade should have a retained clock/gate level, not only a conditional
integer carry. The exact illegal-sequence miscount is a local characterization target. A
nominal implementation may preserve count on reconfiguration unless its modeled gate
creates an edge; do not claim that accepting the write proves that glitch is reproduced.
([Manual §13.6.3, p. 229][aec-notes])

External AEV counting remains available even in standby and wake stabilization; internal
φ counting does not. PWM φW/16 survives watch/subactive/subsleep and their wake
stabilization, but halts in standby; PWM output is high impedance during standby and its
stabilization. Module standby stops the module, while ordinary power-mode halts retain
counts/phase. Counter overflow requests and IRQAEC/IECPWM edge requests remain distinct;
the latter has up to one CPU/subclock cycle synchronization delay. ([Manual table 13.3
and §13.6.6, pp. 228–230][aec-power])

PWM period/duty writes retain count and output. Comparator distances wrap in 16 bits
when a new threshold is below the current count. A period reset that does not change the
output is crossed arithmetically; even duty ≥ period keeps the hidden counter running
while forcing output low. Forced-low and enable selection traverse the same gate/request
rules as clock-driven changes. Live counter-source selection also passes a physical mux
edge through the existing increment path. The next PWM reload uses the new clock;
remaining count survives a disconnected source.

## Live reconfiguration

The manual distinguishes a protected write, a reserved readable/writable bit, and a
programming sequence whose result is not guaranteed. These are different hardware cases.
A guest performing the latter still executes on a real chip. Model off-sequence writes
with register/state transitions and deterministic local inferences. Keep those
inferences next to the affected mechanism, not as a second execution mode.

For every live reconfiguration: settle old-source activity through the bus access,
perform the write, evaluate affected gates/muxes, and project the next event from
retained progress. Do not reset a counter/divider merely because software changed its
configuration.

## Firmware use and further evidence

[`TimerB1Init`](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L35-L44)
stops and configures Timer B1, loads F8, then starts watch/256 reload counting.
Eight input clocks give 62.5 ms at 32.768 kHz. The examined `pw` sources contain no
AEC register programming. AEC behavior therefore relies on the hardware sources and
focused diagnostics.

The [manufacturer leads](../SOURCES.md#located-manufacturer-material) include Timer B1
counting seconds and AEC PWM output. Their examples can add independent configurations
for comparison. Their inclusion in the catalogue does not mean those comparisons have
already been performed.

## Implementation and checks

Implementations are [Timer B1](../../crates/hs-core/src/mcu/timer_b1.rs),
[Timer W](../../crates/hs-core/src/mcu/timer_w.rs) and
[AEC](../../crates/hs-core/src/mcu/aec.rs). Shared phase belongs to [clocks](clocks.md).
Hachiware's [timer cases](https://github.com/lumirth/hachiware/blob/main/cases/timers.py)
exercise live B1 loads, stopped Timer W capture, AEC inputs and selected PWM behavior.
The guest Timer W case checks capture and vector 35; it does not cover all compare,
buffer and access conflicts.

Local [Timer W tests](../../crates/hs-core/tests/timer_w_modes.rs) cover those conflicts,
PWM ties, mode gates, external counting and partition independence.
[AEC tests](../../crates/hs-core/tests/aec.rs) add gate and timing checks.
The case evidence distinguishes documented expectations from chosen live-write rules;
passing the latter protects an inference from accidental change.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[target]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[timer]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=180
[timer-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=185
[aec]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=250
[aec-notes]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=263
[aec-power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=262
