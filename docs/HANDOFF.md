# Implementation work

Read the relevant DESIGN sections, CONTEXT, STATUS and owner research first.
Use the current repository, manufacturer sources, observed behavior and `pw`.

## Verification and checkpoints

```sh
python3 tools/check.py --out out/your-check
python3 tools/verify_retail.py --out out/your-retail
```

Destinations must be new. Standard checks need no private images; the separate
`hachiware` checkout supplies independent diagnostic guests. Retail verification
requires private images, checks they remain unchanged, compares reviewed
workloads and verifies complete event histories and resumed causal state.
The native codec checkpoint passed these checks and Rust 1.95.0 workspace tests;
its local receipts are `out/state-check2`, `out/state-retail` and
`out/state-msrv-tests.log`. Audit corrections have additional named regressions. Boot mode then passed the
full gates and all 120 independent guests in `out/boot-check`, with unchanged
retail observations and native replay in `out/boot-retail`. Comparator routing,
live configuration and RTC free-counter admission then passed the full gates
and all 122 guests in `out/comparator-check`; `out/comparator-retail` confirms
unchanged home/menu workloads and partition/native replay.

Commit and push coherent verified changes regularly. Stage only owned source,
documentation and publishable fixture material. Keep private inputs and their
runtime artifacts ignored.

## Current foundation

The core has one resumable CPU with ordered split accesses, prefetch retention,
exception/interrupt admission and RAM execution. Concrete owners cover the MCU
register map, independent clock sources and prescalers, both timers, RTC, WDT,
ADC, AEC, comparators, GPIO, SSU, SCI/IrDA, IIC2 and flash program/erase/verify.
External EEPROM, sensor and LCD consume actual board transitions.

Power retains RES capacitor charge, independent oscillator startup, qualified
reset release, short-dip logic retention and partial nonvolatile exposure.
Native Borsh save states select causal hardware progress and reconstruct caches
without executing a guest effect. Their API, bounds and exception behavior are
in SAVE_STATES. The exact owner limitations remain in STATUS.

## Remaining work

1. **Manufacturer boot characterization.** The documented protocol now runs
   through existing SCI, flash, clock and GPIO owners and hands arbitrary RAM
   code to the ordinary CPU. Preserve that implementation while improving the
   remaining nominal ROM overhead/physical parameters from new evidence. Its
   tests cover transmitted echoes and final stop, erase/restore, odd uploads,
   invalid lengths, reset interruption and zero ordinary-run allocation.
2. **Owner completion.** Internal flash needs persistent-byte/commit callbacks.
   Comparator gating and external-reference selections now follow the retained
   latch/mux model, with independently routed channel interrupts. Continue to
   model guest-configuration consequences from the connected hardware.
   Extend same-time register and pin conflict coverage at each affected owner.
3. **CPU coverage.** Broaden independently stated encoding, flags and bus timing
   cases beyond the current alias, displacement, divide, prefetch and admission
   diagnostics. Classification totality alone does not certify the ISA. Preserve
   stable issued actions and already-completed effects across every suspension.
4. **Physical response.** Improve sensor filtering/calibration, LCD analog
   response, optical receiver behavior and supply parameters from primary
   sources, successful firmware sequences and discriminating observations.
   Document chosen physical parameters locally and keep the core operational.
   Use separate fitting and validation stimuli for any captured measurements.
5. **Measured performance.** Profile representative retail and custom workloads.
   Candidates include broad MCU synchronization, serial/buzzer edge processing,
   12 kHz sensor publication and decoding. Validate complete outputs before paired
   timing; retain one transition mechanism and portable optimizations. Record
   host, workload, code size and memory rather than extrapolating synthetic gains.
6. **Host tooling.** Replace handwritten report JSON with the justified standard
   serializer when touching report structure. Add generative tests where a real
   behavioral invariant benefits. Rust core completion precedes frontend/C/Wasm
   adapters; presentation helpers should follow a concrete frontend need.

## Updating expectations

`hachiware/spec` and its original guests are independent target expectations.
`workloads/retail.json` is a reviewed software-observed baseline with input
identity and an explicit basis. The verifier never updates it. When a justified
hardware correction changes retail behavior, retain the independent case,
explain the consequence and review the changed observations before updating the
baseline. Capture/restore comparisons use causal state and subsequent effects;
profiler totals and rebuilt caches are not hardware state.
