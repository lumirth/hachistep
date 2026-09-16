# Implementation handoff — after revision 0.2

## Reproduce before changing behavior

```sh
python3 tools/check.py --out out/your-check
python3 tools/mutation_check.py --out out/your-mutations
python3 tools/verify_retail.py --out out/your-retail --menu-trace
```

Destinations must be new. Standard checks need no private images. Private-input
verification checks source hashes are unchanged and now checks explicit reviewed
software expectations. It does not call every changed endpoint a hardware bug.

Read REVISION-0.2, STATUS, `machine.rs::run_until`, then the affected owner and its
integration tests. This revision extends the original Git history; do not add a
second executor, retail-PC shortcuts, raw SFR mirror, or a new device framework.

## What is now closed enough to build on

- The 95-entry register access-width/timing transcription is independently tested;
  RTC, timers, ADC, SPCR and IrCR no longer receive the SSU/SCI three-state cost.
- CPU/SSU/ADC waits retain clock-edge obligations across source/gate changes.
- Dual comparators and the AEC are functional owners, not address scaffolding.
- Timer W captures/buffers/external clocks work, including stopped capture and
  documented register conflicts through actual GPIO routing.
- Aliased predecrement source timing, RTE versus LDC deferral and specific d24
  encoding restrictions are corrected. Dedicated NMI input/latching/masked wake
  and EEPMOV.W break handling are implemented without an alternate executor.
- Strict conformance manifests, complete-output comparison and mutation checks
  provide meaningful constraints for subsequent work.

## Next work, in dependency order

### 1. CPU fetch/microtiming and interrupt admission

Files: `cpu/mod.rs`, `cpu/decode.rs`, `machine.rs`, `tests/cpu_regressions.rs`,
`tests/nmi.rs`, and the independent guest corpus.

Complete discarded/prefetched accesses, phase placement and total timings for
all applicable forms. The new alias/d24 tests certify only their stated cases,
not the whole instruction set. General enable/flag races, short NMI pulses and
synchronizer/admission timing remain. RTE is fixed; do not re-add LDC's delay to
it. EEPMOV.W now abandons the remaining copy on NMI and saves the next PC; retain
that rule while improving bus timing. `next()` must keep an issued action stable.

Acceptance: independent encoding/flags, source-correct ordered bus traces,
phase-swept events, partial effects before reset, self-modification before/after
fetch, typed-state/product-history partition and restoration equivalence.

### 2. Finish source-phase and lifecycle contracts

Files: `mcu/clocks.rs`, `control.rs`, `sci.rs`, timer owners and `machine.rs`.

Extend retained obligations to SCI where applicable. System source switches
still rephase their fractional epoch; establish the actual source-selection and
oscillator stabilization rules before replacing that witness. Source-derived
waits and independently timed external-chip operations are different. Complete
retention matrices without erasing external-device lifetimes on MCU reset.

Acceptance: clock/gate changes at every relevant phase, standby/watch/subactive
transitions, no stale expired visibility appointment after clock changes, NMI
wake/strap tests, and local conflict rules rather than a global priority hack.

### 3. Complete digital hardware gaps

IIC2 and internal MCU flash programming are still missing. Serial slave,
bidirectional/RX-only, synchronous/external-clock SCI and some active
reconfiguration modes remain unsupported. Complete GPIO mux, open-drain and
contention behavior first where those modes depend on it. Resolve the reached
0xF088 bits from target evidence rather than naming a guessed register.

AEC and comparators exist now: extend their existing owners and tests, not new
parallel models. Resolve AEC module-stop ambiguity, clock polarity and gate-edge
apertures; comparator delay/offset/noise and digital-read suppression witnesses.
Timer W clock-mux glitches and exact sub-state input synchronization remain.
ADC triggers/channel-change/retention are incomplete. Implement actual flash
program/erase/verify controls and fetch restrictions, not a direct page-write API.

Acceptance: all applicable control paths through guest accesses, invalid-mode
behavior with honest model errors, pin and register phase sweeps, distinct
hardware/controller latches, preservation across reset/gating, mutation-sensitive
expected observations, and no private-firmware patching.

### 4. Physical sensor, display, supply and optical closure

Files: external owners, `adc.rs`, `comparators.rs`, `machine.rs`.

BMA filter window/rounding/calibration, publication skew, 0x1E effects, thresholds,
wake modes, self-test and alternate interfaces still need work. LCD COM/scan/
column and analog control behavior are provisional. Linear battery conversion,
brownout/reset and optical receiver/transmitter response are not characterized.
Interrupted programming must not become an invented atomic all-old/all-new rule.

Use narrow physical experiments with separate fitting and validation stimuli.
Record observable versus inferred timing and instrument effects. No physical
captures are included in this delivery, and no passing test changes that fact.

### 5. Optimize measured work

The inactive AEC optimization is complete and checked against full menu output
history. Remaining opportunities include broad MCU synchronization, individual
serial/buzzer edges, 3 kHz sensor work and decoding. Exact edge-run/waveform
compression must retain one owner transition mechanism. Use the paired tool's
untimed history preflight and review full typed-state invariants separately.

A performance comparison after an accuracy correction needs a newly justified
behavior baseline. Do not turn off the correction to preserve an old hash.
Report binary/code size, memory, workload and host, not a universal speed claim.

### 6. Integration and durable sessions

The CLI is replay, not a GUI or tested HGSS live peer. Live transport must respect
causality. Portable snapshots must encode every causal latch/continuation/clock
history, not just RAM and EEPROM. Test promised host architectures and declared
MSRV; only the delivery compiler was exercised here. Package private assets
separately from publishable source. Do not install or trigger network CI.

## Updating expectations

`conformance/spec` and documented guest fixtures are independent target
expectations. `conformance/regressions/private-retail.json` is different: a
reviewed software-observed baseline with input identity and an explicit basis
revision. The verifier never updates it. A legitimate hardware change may alter
retail instruction counts, timing, frame hashes or the synthetic motion result;
explain that change before rebasing, and retain the independent case that caused
it. Do not certify correctness by copying the candidate's output into a golden.
