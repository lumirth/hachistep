# Clocks and operating modes

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The model retains oscillator lifetimes, source phase, shared prescalers and peripheral
clock obligations. Active, sleep, watch, subactive, subsleep and standby apply their
documented clock and retention rules. Direct transitions pass through the intermediate
mode and count stabilization in oscillator cycles. A stopped source suspends its
remaining work. Clock outputs pass through the pin mux and can affect connected chips.

## Limits and open questions

Renesas's target manual and A287 amendment, together with `pw` clock transitions,
support the clock tree and mode behavior. Startup uses selected delays from electrical
tables. An electrical maximum used as the nominal delay does not establish when an
actual oscillator first supplies usable edges. Divider reset polarity, mux-induced
edges and some reconfiguration phases remain local circuit inferences. Review these
when a sequence depends on the first edge after reset, wake or a source change.

## Sources and applicability

- Renesas [H8/38602R hardware manual, REJ09B0152-0300, rev. 3.00][manual]:
  clock tree/registers/prescalers §§4.1–4.4, printed pp. 63–71; mode transitions
  §§5.1–5.3, pp. 78–94. PDF page numbers are printed page numbers plus 34.
- [TN-H8*-A414A/E, addition of H8/38606][addition], 2009-04-22, pp. 1–5:
  target differences concern memory, package, and flash organization; it does not
  replace the clock chapters.
- [TN-H8*-A287A/E, specification changes][clock-update], 2004-11-10, p. 1:
  amended SYSCR1 stabilization guidance and STS table. Rev. 3 retains these values.

## Clock tree and transitions

| Area | Relevant facts and exact manual location |
| --- | --- |
| Sources | E7_2 selects the main oscillator at reset; OSCF is read-only. SUBSEL selects crystal watch clock or Rosc/32; SUBSTP stops the subclock oscillator. RFCUT controls feedback resistance, not clock selection. [§4.1.1, fig. 4.1, §4.2.4][clock-registers]. |
| Prescalers | S resets/stops in standby, watch, subactive, subsleep; W stops in standby but continues through watch/subactive/subsleep. [§4.4][prescalers]. |
| Transitions | SA changes take effect through SLEEP. Direct transitions include an intermediate sleep/watch state; I=1 prevents the direct-transition exception. Subactive→active includes STS delay counted in oscillator cycles, before destination-clock exception cycles. [§5.3, especially equation 6][direct]. |

The STS selectors `000…111` correspond to `8192, 16384, 1024, 2048, 4096, 256, 512, 16`
oscillator states. The update recommends `111` for an external/on-chip source and
explicitly notes different early-start behavior for other settings; a generic
divided-CPU delay does not express that distinction. [Clock update, p. 1][clock-update].

## Firmware clock transitions

[`ClockSleep`][pw-sleep] writes SYSCR1/SYSCR2 then executes SLEEP.
[`CaptureSample`][pw-sample] uses `0xa7/0xeb` for a direct return from subactive
operation: STS=`010`, therefore 1024 oscillator states. The firmware therefore exercises
the 1024-state stabilization wait.

## Shared divider phases

Prescaler S has the §4.4.1 reset/stop domain, and W retains its §4.4.2 standby phase
independently of the upstream phiW/4 divider. Each output keeps its emitted-edge ordinal
across reset; phase reset cannot rewind a peripheral's consumed work. CPU reference
selection is separate from main phi. Timer W's input synchronizer and SSU
holding-register load use that CPU reference; main-clock peripherals do not inherit the
subactive CPU's watch frequency.

For undocumented reset polarity, the selected circuit uses high-first divider outputs,
consistent with the Timer W timing drawing. This is separate from the documented zeroed
up-counter state: the drawing does not identify Q versus /Q. AEC gate transitions use
the physical divider phase. A Timer W internal source switch from low to high produces
the extra count described in §10.7(3), pp.181–183. No reset transition is counted
through a held consumer.

## Oscillator controls

OSCCR writes retain SUBSTP, RFCUT and SUBSEL; OSCF remains the board's read-only
main-oscillator strap. SUBSTP stops X1 alone; SUBSEL routes ROSC/32 through the watch
domain even with X1 stopped. RFCUT is latched at the specified low-power transition; the
prescribed oscillator frequency is still the physical input to this digital model,
rather than an analog feedback-resistor simulation.

Source lifetimes follow §5.5. ROSC stops when WDT, reset and the subclock generator all
release it. Stopped sources retain their emitted count and create no virtual elapsed
edges. Restart establishes a fresh source phase. Consumers retain their unfinished edge
obligations, including a partially transmitted watch-clock SCI character. Source
reconfiguration refreshes clock projections without restarting the peripheral operation.
Programmed SYSCR divisors still latch through SLEEP.

Table 5.3, printed p.86, specifically lists the subclock oscillator as functions/halted
in standby: X1 remains under SUBSTP control. Prescaler W and watch consumers halt
independently, including standby wake stabilization. This preserves X1's phase through
standby when it was left enabled. The ROSC-backed watch generator follows whether the
shared ROSC still has a consumer.

## Implementation and checks

The [clock implementation](../../crates/hs-core/src/mcu/clocks/) and
[mode control](../../crates/hs-core/src/mcu/control.rs) own the source and mode state.
Hachiware's [clock cases](https://github.com/lumirth/hachiware/blob/main/cases/clocks.py)
exercise direct transitions, masked transitions and ROSC/watch selection.
[Clock-obligation tests](../../crates/hs-core/tests/clock_obligations.rs) check source
phase, stopped sources and unfinished peripheral work. These support digital clock
behavior. [Power and reset](power-and-reset.md) describes the separate approximation
of oscillator availability after supply loss and the remaining warm-start questions.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[clock-update]: https://www.renesas.com/en/document/tcu/h838602-group-specification-changes#page=2
[clock-registers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=97
[prescalers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=105
[direct]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=125
[pw-sleep]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/lib_common.c#L658-L672
[pw-sample]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_main.c#L517-L523
