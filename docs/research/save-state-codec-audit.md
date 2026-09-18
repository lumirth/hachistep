# Native codec source audit

2026-09-18, current uncommitted starter implementation. Read-only comparison of
`machine/state.rs`, `cpu/state.rs`, clock-wait serialization and owner validation
with the [codec contract](save-state-codec-contract.md),
[CPU progress mapping](save-state-cpu-progress.md) and
[MCU validation inventory](save-state-mcu-validation.md).

All three concrete findings below have been corrected: the RTC retains bit 7,
watchdog validation relates its private divider to actual ROSC edges, and an
active completed SSU shifter is accepted only in an explicitly faulted session.
Named regressions cover all three. The references below identify the code reviewed, before
those corrections; line numbers can move with the fixes. No production/test
files were changed and no build or test was run for this audit.

## 1. Valid RTC free-counter state fails decoding

The reviewed `Rtc::validate` rejects both `flags & 0x80` and
`control2 & 0x80` at `crates/hs-core/src/mcu/rtc.rs:267–269`.
Both bits are reachable: the write handler stores the entire byte at `0xf06d`,
and free-counter overflow executes `flags |= control2 & 0x80`.
([Validator and runtime](../../crates/hs-core/src/mcu/rtc.rs).)

Reproducer conditions:

1. Custom firmware selects an RTC free-counter source, selector 0–7.
2. Write `0x80` to RTCCR2 at `0xf06d`, and enable the counter through RTCCR1.
3. Capture and encode. Decoding already fails because control2 contains bit 7.
4. Let the 8-bit free counter overflow; a capture now also contains valid
   `flags == 0x80`, which independently fails the same validation.

This is a normal guest-reachable state, without malformed input. Remove the
two high-bit prohibitions. Keep the separate `CalendarUpdate.flags & !0x7c`
check: staged calendar flags and the complete visible flag register have
different ranges. The implementation agent accepted this correction.

## 2. Watchdog fixed headroom permits an overflowing private divider

The reviewed validator only checks `rosc_ticks.checked_add(65536)` and
independent scalar headroom (`watchdog.rs:209–219`). Runtime synchronization
adds the actual elapsed ROSC interval, which can be much larger:

```text
cycles = raw.saturating_sub(rosc_last) + rosc_phase
rosc_ticks += cycles / 2048
```

These are unchecked additions at `watchdog.rs:75–76`.
([Watchdog runtime and validation](../../crates/hs-core/src/mcu/watchdog.rs).)

A bounded internal codec regression can start with a valid machine at roughly
200 seconds, retaining an available ROSC, its module gate enabled, and the
watchdog counter disabled (`control1 == 0xaa`, selector 0). Change the decoded
private fields to `rosc_last = 0`, `rosc_phase = 0`, and
`rosc_ticks = u64::MAX - 65536`, leaving the other fields valid. Encode the
candidate with the normal envelope/checksum, then decode it.

The local validator accepts those values. The watchdog deadline is absent
because the counter is disabled, so MCU appointment validation does not reject
the ancient divider cursor. Nevertheless, `rosc_required()` remains true
because the module gate is enabled. At the first subsequent MCU sync, about
128,000 private divider ticks are added, overflowing `rosc_ticks`. Overflow
checks panic; an unchecked build wraps the causal divider ordinal.

Validate the exact accumulation against the restored ROSC ordinal and capture
time before accepting the candidate. The counter-enable state must not bypass
validation of a still-running private oscillator divider. The implementation
agent accepted this correction; checking a fixed extra margin alone does not
establish it.

## 3. SSU permits a live external frame already beyond its completion edge

The reviewed `Ssu::validate` allows `edges == 16` whenever `next` is absent
(`ssu.rs:512–518`). That is appropriate for a completed inactive shifter, but
also admits an active externally clocked frame. Its clocking does not use an
internal `next` appointment.
([SSU source](../../crates/hs-core/src/mcu/ssu.rs).)

Reproducer conditions:

1. Start from a captured, selected, externally clocked active SSU frame with
   gate open, `phase == Edge`, and `next == None`.
2. Change only its decoded `edges` to 16; leave the session's fault flag clear.
3. Decode the normally enveloped candidate. The reviewed validation accepts it.
4. Supply an external clock transition without deselecting the device.
   `input_pins` calls `shift_edge`, increasing the count to 17.
5. `finish_edge` only completes at **exactly** 16, so the active frame misses
   retirement. Continuing transitions eventually overflows the `u8` count
   after 240 transitions from the captured state.

The relevant runtime is `input_pins` at lines 446–463, the unchecked increment
at line 437, and exact completion comparison at line 489. Machine board
resolution calls `finish_edge` after sampling, so normal operation does not
leave a healthy externally driven frame at this state.

Reject an active Edge-stage frame with count 16 in a healthy decoded session.
An explicitly faulted capture may have stopped between final edge generation
and board completion; preserving that stopped session requires a separate
exception rather than accepting the same shape as runnable. Do not reject
inactive completed counters or stale counters awaiting the next Load. The
implementation agent accepted this correction and the faulted-state distinction.

## Contract checks with no further finding

The reviewed saved machine includes the causal board fields, firmware origin,
full device state, pending bus lane and lifecycle waits. Restore creates a
candidate, validates it and recomputes the next appointment without board
resolution. It does not re-emit chip-select, serial or pin effects.
([Machine codec](../../crates/hs-core/src/machine/state.rs).)

The CPU mapping retains fetched words and prefetch data, admission progress,
latched memory/bit/arithmetic results, exception stack stages, deferred address
updates, all five EEPMOV substeps and absolute-8 MOV provenance. Pending request
matching uses the pure `issued_action` projection. No missing causal field or
repeated completed CPU effect was identified in this bounded comparison.
([CPU codec](../../crates/hs-core/src/cpu/state.rs),
[CPU runtime](../../crates/hs-core/src/cpu/mod.rs).)

Clock-wait decode explicitly invalidates skipped timestamp-cache revisions;
it does not mistake a zero timestamp for a valid reconstructed projection.
The remaining lazy-wait distinction is documented in the prior inventory:
past ADC settling/capture-visibility projections can be legitimate, and their
source epoch must remain reconstructible. No additional current Machine path
retaining an expired lazy wait across a source rebase was identified.
([Clock waits](../../crates/hs-core/src/mcu/clocks.rs).)
