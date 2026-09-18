# Comparator live configuration

Research handoff, 2026-09-18. Scope: the two guest-configuration faults in the
current [comparator owner](../../crates/hs-core/src/mcu/comparators.rs), plus a
directly related interrupt-routing defect. No production/test edits or builds.
This follows DESIGN §§1.1 and 10.9: physical inference supplies deterministic
behavior where programming guidance does not specify the out-of-sequence case.

## Primary evidence

The H8/38606 addition lists the target's differences from H8/38602R; it makes
no comparator change. Relevant authority remains hardware manual
REJ09B0152-0300 §§5.1.3, 5.4 and 18. ([A414A target addition][target].)

The manual's Figure 18.1 separates the reference selector, comparator, result
register and interrupt generator. `COMPCKSTP=0` places the module in standby;
§18.5 advises clearing CME first. Table 18.2 labels `CMR=CMLS=1` prohibited.
Neither passage describes an exception, rejected bus write or reset. Comparator
operation survives **LSI** watch/standby when enabled; that differs from
explicit **module** standby. ([Block/register rules][manual-comparator],
[module gate][module-gate], [module standby][module-standby],
[usage notes][usage].)

The internal-reference application note explicitly uses VIH without
hysteresis and says CRS has no effect with external reference. This resolves
the hardware manual's contradictory VIL sentence. The external-reference
example selects PMR3.VCref before CME and waits 15 µs. Both retain ordinary
read-to-arm/read-then-clear flag behavior. ([Internal example, §4.3][internal],
[external example, §§4.3–5.1][external].)

The A287A correction's comparator characteristics specify 15 µs as the maximum
conversion time, not a measured typical value. Keep the existing 15 µs
canonical response parameter; no new timing knob is needed. ([Correction,
Table 21.8][correction].)

The matching `pw` checkout at `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`
contains no comparator-configuration sites found by the source search. Its
battery measurement uses ADC channel 7. It supplies no stronger observation
for the two unusual sequences. Its comparator vector declarations are useful
independent corroboration below. ([BatterySample][pw-battery],
[vector declarations][pw-vectors].)

## Adopt these compact behavior rules

### Module gating with CME still set

**Chosen physical inference:** the module gate suppresses operation without
resetting its stored controls and digital latches. Removing bias discards an
unfinished analog response; restoring operation starts a fresh response.
This implements the module's documented stop control while respecting the
separate register/latch structure. It does not pretend that the recommended
software sequence is enforced by hardware.

| Transition or operation | Required behavior |
| --- | --- |
| Write CKSTPR2.COMPCKSTP from 1 to 0 | Complete comparator work already due at the write boundary, accept the bit, cancel both pending comparison deadlines and clear their settling markers. |
| State retained while gated | CMCR0/1 bytes; each last CDR; CMF; interrupt baseline and armed latch; flag-read qualification. Do not clear CME or synthesize CMF. |
| Analog changes while gated | No CDR transition, new CMF or comparator IRQ. Continue retaining actual board inputs through their normal owner; do not queue every missed crossing. |
| CMCR write while gated | Store the entire byte. No analog deadline until the module is enabled. Explicit CME/CMIE clearing still disarms the interrupt latch, as an ordinary control write does. |
| CMDR read/write while gated | Read retained CDR/CMF. Preserve flag read-then-zero clearing. A read cannot newly arm comparison while the module is stopped. |
| Gate returns with CME=1 | Evaluate current inputs under retained control, retain CDR meanwhile, and schedule one full response interval from the return instant. Do not resume the discarded interval. |
| Gate returns with an existing CMF | Retained enabled request becomes eligible immediately; it is not a new comparison. A later settled result can generate another flag through the ordinary baseline rule. |

Retain the existing same-time CMDR read arbitration. Physical supply loss,
reset and module gating remain their existing distinct lifetime operations;
this change must not turn a gate write into a board power cycle.

The current `set_gate` already performs almost all of this after its
`Unsupported` branch: `reevaluate(..., false)` cancels work on closure, and
`reevaluate(..., true)` restarts enabled channels on reopening. Remove the
guest fault, retain those latches, and exercise the rules above.

### External reference together with CMLS

**Chosen physical inference:** external reference wins the input mux. Internal
hysteresis works by choosing ladder taps; bypassing that ladder leaves one
external comparison threshold. Therefore `CMR=1,CMLS=1` behaves electrically
like external non-hysteresis, while CMLS and CRS remain readable stored bits.
The documented mux and explicit external disabling of CRS support this
choice; it does not invent an external hysteresis voltage or silently rewrite
the guest's control byte. ([Reference selector][manual-comparator],
[CRS meaning][internal].)

For every channel, use the existing integer comparison:

| Effective selection | Target result |
| --- | --- |
| CMR=1, either CMLS | `COMP > VCref`; equality is low. CRS does not participate. |
| CMR=0, CMLS=0 | `30 × COMP > (11 + CRS) × Vcc`. |
| CMR=0, CMLS=1, last CDR=0 | Same upper threshold `(11 + CRS)/30`. |
| CMR=0, CMLS=1, last CDR=1 | Lower threshold `(9 + CRS)/30`. |

These thresholds use the committed CDR as the hysteresis history, not the
pending target. The existing `desired()` already gives CMR priority; removing
the rejected combination reaches that path. PMR3/P30 selection remains board
state: a CMCR write must not secretly rewrite the port mode register.

Make restart decisions from **effective analog configuration**, not every
changed control bit:

- CME rising or module restart starts stabilization.
- A live internal/external reference selection change, or an effective
  internal threshold/hysteresis setting change, replaces pending work with a
  response under the new configuration.
- A CMIE-only write changes interrupt admission, not analog progress.
- With CMR remaining 1, changing only CMLS/CRS stores the bits without changing
  the target or its deadline. They are disconnected selections. In particular,
  repeated writes cannot postpone a real external crossing indefinitely.
- An identical control write leaves the response phase intact.

The existing `(old ^ new) & 0xbf` restart test does not satisfy the ignored-bit
rule; it needs the small effective-reference distinction. Clearing CMR later
makes the retained CMLS/CRS effective again and uses the normal live-change
response. No additional queue, oscillator or execution path is needed.

For both changes, preserve the existing inertial response: a transient input
that returns before the pending transition completes cancels that transition;
a changed desired target receives a new response deadline. Keep actual time
durations, because comparators continue through stopped CPU/source clocks.

## Correct the two channel interrupt routes

This finding is directly documented. Table 3.1 assigns COMP0 vector 21 at
`0x002a` and COMP1 vector 22 at `0x002c`; vector 36 at `0x0048` is reserved.
([Vector table, printed pp.42–43][vectors].) `pw` declares exactly those two
comparator handlers. ([Matching declarations][pw-vectors].)

The current [MCU integration](../../crates/hs-core/src/mcu/mod.rs) combines
`interrupt_with_enable([retained[7], retained[8]])` into `push(36)` around
lines 415–419. Return a two-bit channel request mask, or an equivalent channel
selection, preserving each channel's retained CMIE handling. Admit `21+i` for
each asserted channel through the existing controller priority calculation.
Both asserted flags must survive selecting the higher-priority channel.

CMDR reads still arm/update each comparator baseline only when that channel
is operating with CME and CMIE set. Clearing CMIE disarms without clearing
CMF. Merely enabling CMIE must not invent a read strobe. The normal CMF
read-then-zero clear and same-time read masking remain unchanged.

## Five discriminating fixture ideas

Use short custom firmware with explicit analog inputs and capture register
reads/interrupt-entry vectors. Disable unrelated wake sources. Expected
observations come from the rules above; do not derive expected values from
the owner's internal fields. The first two fixtures distinguish the adopted
inferences and should retain that basis in their descriptions.

1. **Stop during a pending response.** At 3.0 V supply and VCref=1.5 V, arm
   external COMP0 with a low baseline, raise COMP0 to 2.0 V, then close the
   module gate five microseconds into the 15 µs response. Wait 100 µs and
   reopen without changing CME. CDR stays low during the gap; control bytes
   survive; its transition/CMF occurs one full response after reopening.
   This separates retained-latch/fresh-response behavior from reset,
   continued hidden operation and resumed remaining-time behavior. Repeat
   the stop after CMF is already set to distinguish retention from flag loss.

2. **External ignored-bit writes during a real crossing.** Select PMR3.VCref,
   set CMCR0 to `0xbf`, and use VCref=1.5 V with COMP0 initially 2.0 V.
   After settling, lower COMP0 to 1.0 V. During the pending response, rewrite
   only CMLS/CRS using `0xa0`, `0xaf`, `0xb5`; readback follows each write,
   but CDR changes at the original deadline. Then clear CMR: the retained
   internal settings become operative. This detects rejected writes,
   coerced readback, an invented external hysteresis band and deadline
   starvation through ignored bits.

3. **Live mux change and hysteresis history.** At Vcc=3.0 V and CRS=8,
   internal thresholds are 1.9/1.7 V. Keep COMP0=1.8 V and begin internal
   non-hysteresis with CDR=0. Select external VCref=1.5 V: CDR becomes 1
   after the response. Return to internal hysteresis while COMP remains
   1.8 V: the high state remains. Drop to exactly 1.7 V, then return to
   1.8 V: it becomes and stays low; exceeding 1.9 V restores high. A CMIE
   toggle during an input response must not delay that response.

4. **Independent vectors and read qualification.** Set distinct RAM markers
   in handlers 21 and 22, plus a failure marker at reserved vector 36. Enable
   CMIE but omit the CMDR arming read: a crossing must not generate an armed
   request. Read CMDR, cross only COMP1 and expect vector 22. Then assert both
   channels and expect the controller to select 21 before 22 while preserving
   the other flag. Clear one CMF using the documented read/zero sequence;
   the other flag stays set. Include a CMDR read coincident with completion
   to distinguish read masking from a spurious request.

5. **LSI standby is not module standby.** With COMPCKSTP and CME left set,
   start a response and enter watch or standby before its due time. A
   comparator transition still completes after the same physical interval;
   with a read-armed CMIE it uses its channel's wake vector. Repeat with
   COMPCKSTP cleared and expect no transition/request until module restart.
   This catches accidental dependence on CPU edges and conflation of the two
   stop mechanisms.

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
