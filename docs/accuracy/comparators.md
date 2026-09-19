# Comparators

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

Both comparators implement internal ladder and external reference selection, hysteresis,
settling, result latches, read-armed interrupt baselines and separate vectors. Renesas's
internal/external-reference examples resolve conflicting manual prose. A comparator
can continue while CPU clocks stop, subject to its own enable and power.

## Limits and open questions

The model uses the specified 15 µs maximum as its nominal response time. Short transients
are filtered by that response model. Gate restoration starts a fresh analog response;
external reference with the prohibited hysteresis bit selects the external threshold.
Those transient and off-sequence behaviors are circuit inferences. `pw` provides no
reached comparator configuration to strengthen them. Timing-sensitive custom firmware
and narrow analog pulses are the relevant review cases.

## Primary evidence

The H8/38606 addition lists the target's differences from H8/38602R; it makes no
comparator change. Relevant authority remains hardware manual REJ09B0152-0300 §§5.1.3,
5.4 and 18. ([A414A target addition][target].)

The manual's Figure 18.1 separates the reference selector, comparator, result register
and interrupt generator. `COMPCKSTP=0` places the module in standby; §18.5 advises
clearing CME first. Table 18.2 labels `CMR=CMLS=1` prohibited. Neither passage describes
an exception, rejected bus write or reset. Comparator operation survives LSI
watch/standby when enabled; that differs from explicit module standby. ([Block/register
rules][manual-comparator],
[module gate][module-gate], [module standby][module-standby],
[usage notes][usage].)

The internal-reference application note explicitly uses VIH without hysteresis and says
CRS has no effect with external reference. This resolves the hardware manual's
contradictory VIL sentence. The external-reference example selects PMR3.VCref before CME
and waits 15 µs. Both retain ordinary read-to-arm/read-then-clear flag behavior.
([Internal example, §4.3][internal],
[external example, §§4.3–5.1][external].)

The A287A correction's comparator characteristics specify a maximum conversion time of
15 µs. The model uses this bound as its nominal response delay. ([Correction, Table
21.8][correction].)

The matching `pw` checkout at `6dc7bc09950078fa3fe0dffa4dae34e9549a99da` contains no
comparator-configuration sites found by the source search. Its battery measurement uses
ADC channel 7. It supplies no stronger observation for off-sequence configuration. Its
comparator vector declarations are useful independent corroboration below.
([BatterySample][pw-battery],
[vector declarations][pw-vectors].)

## Live configuration

### Module gating with CME still set

Chosen physical inference: the module gate suppresses operation without resetting its
stored controls and digital latches. Removing bias discards an unfinished analog
response; restoring operation starts a fresh response. This follows the module's
documented stop control and separate register/latch structure.

| Transition or operation | Required behavior |
| --- | --- |
| Write CKSTPR2.COMPCKSTP from 1 to 0 | Complete comparator work already due at the write boundary, accept the bit, cancel both pending comparison deadlines and clear their settling markers. |
| State retained while gated | CMCR0/1 bytes; each last CDR; CMF; interrupt baseline and armed latch; flag-read qualification. Do not clear CME or synthesize CMF. |
| Analog changes while gated | No CDR transition, new CMF or comparator IRQ. Continue retaining actual board inputs through their normal owner; do not queue every missed crossing. |
| CMCR write while gated | Store the entire byte. No analog deadline until the module is enabled. Explicit CME/CMIE clearing still disarms the interrupt latch, as an ordinary control write does. |
| CMDR read/write while gated | Read retained CDR/CMF. Preserve flag read-then-zero clearing. A read cannot newly arm comparison while the module is stopped. |
| Gate returns with CME=1 | Evaluate current inputs under retained control, retain CDR meanwhile, and schedule one full response interval from the return instant. Do not resume the discarded interval. |
| Gate returns with an existing CMF | Retained enabled request becomes eligible immediately; it is not a new comparison. A later settled result can generate another flag through the ordinary baseline rule. |

Retain the existing same-time CMDR read arbitration. Physical supply loss, reset and
module gating remain their existing distinct lifetime operations; a gate write does not
power-cycle the board.

### External reference together with CMLS

Chosen physical inference: external reference wins the input mux. Internal hysteresis
works by choosing ladder taps; bypassing that ladder leaves one external comparison
threshold. Therefore `CMR=1,CMLS=1` behaves electrically like external non-hysteresis,
while CMLS and CRS remain readable stored bits. The documented mux and explicit external
disabling of CRS support this choice; it does not invent an external hysteresis voltage
or silently rewrite the guest's control byte. ([Reference selector][manual-comparator],
[CRS meaning][internal].)

For every channel, use the existing integer comparison:

| Effective selection | Target result |
| --- | --- |
| CMR=1, either CMLS | `COMP > VCref`; equality is low. CRS does not participate. |
| CMR=0, CMLS=0 | `30 × COMP > (11 + CRS) × Vcc`. |
| CMR=0, CMLS=1, last CDR=0 | Same upper threshold `(11 + CRS)/30`. |
| CMR=0, CMLS=1, last CDR=1 | Lower threshold `(9 + CRS)/30`. |

These thresholds use the committed CDR as the hysteresis history, not the pending
target. PMR3/P30 selection remains board state: a CMCR write must not secretly rewrite
the port mode register.

Make restart decisions from effective analog configuration, not every changed control
bit:

- CME rising or module restart starts stabilization.
- A live internal/external reference selection change, or an effective
  internal threshold/hysteresis setting change, replaces pending work with a
  response under the new configuration.
- A CMIE-only write changes interrupt admission, not analog progress.
- With CMR remaining 1, changing only CMLS/CRS stores the bits without changing
  the target or its deadline. They are disconnected selections. In particular,
  repeated writes cannot postpone a real external crossing indefinitely.
- An identical control write leaves the response phase intact.

Clearing CMR makes the retained CMLS/CRS effective again and starts a response under
that configuration.

For both changes, preserve the existing inertial response: a transient input that
returns before the pending transition completes cancels that transition; a changed
desired target receives a new response deadline. Keep actual time durations, because
comparators continue through stopped CPU/source clocks.

## Interrupt routing

Table 3.1 assigns COMP0 vector 21 at `0x002a` and COMP1 vector 22 at `0x002c`; vector 36
at `0x0048` is reserved. ([Vector table, printed pp.42–43][vectors].) `pw` declares
exactly those two comparator handlers. ([Matching declarations][pw-vectors].)

Each channel requests its own vector through the ordinary interrupt controller.
Selecting the higher-priority channel leaves the other channel's flag pending.

CMDR reads still arm/update each comparator baseline only when that channel is operating
with CME and CMIE set. Clearing CMIE disarms without clearing CMF. Merely enabling CMIE
must not invent a read strobe. The normal CMF read-then-zero clear and same-time read
masking remain unchanged.

## Implementation and checks

The [comparator implementation](../../crates/hs-core/src/mcu/comparators.rs) owns
response and latches. Hachiware's
[comparator cases](https://github.com/lumirth/hachiware/blob/main/cases/comparators.py)
check read-armed wake, channel priority and live external-reference gating.
Local [comparator tests](../../crates/hs-core/tests/comparators.rs) cover gating and
configuration transitions. Checks using the selected 15 µs response protect that model;
the source supplies a maximum, not every physical unit's transient response.

[target]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[manual-comparator]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=395
[module-gate]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=116
[module-standby]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=130
[usage]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=402
[internal]: https://www.renesas.com/en/document/apn/h838602r-group-application-note-voltage-comparison-comparator-internal-voltage-reference#page=9
[external]: https://www.renesas.com/en/document/apn/h838602r-group-application-note-voltage-comparison-comparator-external-voltage-reference#page=9
[correction]: https://www.renesas.com/en/document/tcu/h838602-group-specification-changes#page=11
[vectors]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=76
[pw-battery]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_battery.c#L47-L86
[pw-vectors]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_intprg.c#L37-L45
