# Manufacturer boot-service audit

Reviewed 2026-09-18. Scope: the current `machine/boot.rs`, its machine bus/reset
integration, and native-state validation. This was source inspection, without
running builds or tests. No earlier HachiStep implementation was consulted.

## Primary contract

Renesas §6.3.1 and Table 6.2 specify autobaud from serial zeroes, a zero response,
host `55`, whole-flash blank checking/erasure, `AA` success or `FF` erase failure,
two echoed length bytes, echoed payload into `FB80..FF7F`, then final `AA` and
execution of the uploaded program. Handoff disables SCI transmit/receive while
retaining BRR and driving TXD high. The target addition expands flash to six erase
blocks; it does not enlarge the documented boot upload aperture.
([Hardware manual, pp.105–108](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=139),
[H8/38606 addition, flash block configuration](https://www.renesas.com/en/document/tcu/addition-h838606-group))

The ordinary implementation follows that sequence. Its erase sequence uses the
existing flash owner, selects one block at a time, performs the documented setup,
pulse, recovery and verify waits, and limits retries to 100. The WDT is enabled
only around each pulse with a roughly 19.8 ms guard; the 10 ms pulse and setup
complete before that guard in the documented clock range. Verification uses an
aligned byte dummy write followed by both word reads. These agree with §6.4.2 and
Figure 6.4. ([Hardware manual, pp.112–113](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=146))

## Findings addressed during this review

1. **Payload continuations bypassed upload bounds.** The former
   `Boot::validate` checked direct receive/store stages but omitted
   `Send`, `SendStatus` and `SendByte` carrying
   `AfterSend::Receive(Payload)`. An invalid decoded length/cursor could therefore
   survive validation, finish an echo, and resume stores beyond the upload RAM
   aperture. The current validation at `boot.rs:435` applies the same
   `1..=1024` length and `cursor < length` constraints to those continuations.
   This prevents an invalid file from driving later MMIO writes through the
   unchecked `FB80 + cursor` expression.

2. **A verify cursor could belong to a different block.** The former global
   alignment/flash-size checks allowed a verify cursor beyond its selected
   block's end. Since completion detects the end by equality, it could miss that
   endpoint and continue dummy writes and reads outside the selected range,
   producing extra effects and spurious failure/retry behavior. The current
   validation at `boot.rs:425` requires each active `Verify*` cursor to lie within
   its selected block. `VerifyEnd` correctly remains exempt: completion may have
   advanced the cursor to the block's exclusive end. `Pulse` also overwrites its
   previous address before verification, so that inactive address needs no
   selected-block restriction.

3. **Erase admission retained unchecked future block progress.** The parent
   independently found that `FlashBegin` could accept block 6 and later enter a
   pulse for a nonexistent erase block. The current `boot.rs:396` requires
   block/attempt zero at this admission stage; accepted confirmation also resets
   that progress. Its old values no longer become future block indices.

These fixes were read back in the current source. Their execution checks remain
the parent implementation task's responsibility.

## Integration conclusions

Boot accesses use the ordinary pending-bus request and clock obligation. Native
restore compares the pending request against `Boot::action`; it neither invokes
`next` nor repeats a bus effect during loading. Host call boundaries only observe
the autobaud pin level: the first low interval retains its source ordinal, and
later observations of that same low level do not restart measurement.

External reset and supply interruption discard the service and its pending bus
request while the existing owners settle retained flash/device state. WDT reset
uses the existing ROSC reset hold and resumes user reset rather than resampling
the boot strap. Final handoff waits for the SCI shifter and holding register to
empty, so it includes the complete final stop bit before disabling transmission.
The boot owner prevents interrupt admission until handoff, consistent with
§6.4.3. No further concrete protocol, erase/WDT timing, or repeated-effect defect
was found within this scope.
