# Revision 0.2 — timing and functional hardware expansion

This revision continues the delivered starter at
`69f952d87bae3fd79e58c4ae4f72dfe69667af47`. The included `.git` retains that
history and the individually committed fixes and new owners. It is still a
research/development core, **not a completed silicon-indistinguishable emulator**.

## Implemented changes

| Area | Concrete result |
|---|---|
| Register bus | Corrected two/three-state access classification and native AEC words; 95 independently transcribed register expectations and timed guest-access tests. |
| Clock work | CPU physical/internal work, SSU half-edges and ADC sample/result phases retain remaining clock-edge obligations. Source changes and downstream gating no longer leave those waits at stale absolute times. |
| GPIO | Restored two pull-up writes lost inside a source comment. Added input-only package fixtures, Timer W/AEC routes and the correct P30 VCref reference route. |
| Comparators | New dual-channel owner with ladder/external reference, hysteresis, read-armed comparison baseline, qualified flag clearing, module/reset behavior and vectors 21/22. A real diagnostic program sleeps and wakes on a supplied analog crossing. |
| Timer W | Paired compare/capture buffers, stopped-counter capture, external FTCI, local read/write/capture/buffer conflicts, equal PWM-match handling and stabilization gating. Guest GPIO capture takes vector 35. |
| AEC | New independent 8-bit/cascaded 16-bit counters, internal/external clocks, selected edges, gate-return effects, PWM gating/output and separate hardware flags/controller requests. Guest diagnostics exercise vectors 18 and 32. |
| CPU | Corrected aliased predecrement stores at all widths, RTE versus LDC interrupt delay and reserved d24 selectors. Added NMI latching/masked wake and EEPMOV.W interruption between complete transfers; .B defers NMI. |
| Verification | Fifteen core-independent guest fixtures, strict input-hashed/provenance manifests, explicit software retail regressions, full exported-state/product-history comparison and isolated mutation checks. |

The details and source sections are in [the engineering record](NEXT-REVISION.md).
Tests use the production executor and owners. There is no PC hook, native
firmware replacement, second interpreter, rollback executor or accuracy mode.

## Validation scope

The standard Rust configuration has **110 passing checks including the doctest**
and one private-input test that is separately opted in. The same checks are run
in debug, release, and all-features/trace configurations. There are **17 Python
checks**, **15 guest fixtures**, and **3 deliberately broken implementation
mutations** whose unmodified controls pass and compiled mutants fail. Formatting,
Clippy with warnings denied and an extracted-package rebuild are release gates.
Actual logs and toolchain identities are preserved in `evidence/next-revision`.

The private Rust replay compares complete typed machine state and complete
product-event vectors across arbitrary run partitions and snapshot restoration.
No physical hardware was connected. Passing a test here does not certify an
unmeasured sensor transfer function or undocumented silicon timing.

### Unmodified supplied firmware

| Workload | Observed result after the corrections |
|---|---|
| 10-second home boot | 6,205,679 retired instructions, 237 interrupt entries; visible home screen. |
| 6.5-second button replay | 5,426,069 retired instructions, 464 interrupt entries; menu selection and 688 buzzer transitions. |
| 61-second synthetic acceleration | 16,779,551 retired instructions, 2,116 interrupt entries; firmware displays 107 steps. |
| 120-second stationary session | 21,933,098 retired instructions, 3,148 interrupt entries; display enters power save. |

All four reach their exact requested horizon without a model fault. Firmware and
EEPROM input hashes remain unchanged. The step count is a **software-model
regression observation**, not an independent physical pedometer expectation.

`verify_retail.py` now checks scenario identity, semantic report values and all
six exported-image hashes against an explicitly labeled baseline. It never
updates that baseline. For new images, `--smoke-only` explicitly forgoes the
behavioral comparison; it does not change core execution.

## Measured, behavior-preserving optimization

The AEC initially recalculated inactive clock state at unrelated MCU reads.
The optimized owner holds inactive state and rejoins shared phases on enable;
its tests ensure elapsed gated clocks are not replayed. Duplicate pin-resolution
calls at the same timestamp also avoid redundant work.

In the initial four ABBA/BAAB blocks (eight samples per binary), the 6.5-second
menu workload's median simulation time fell from **1.037671 s to 0.883850 s**,
**14.82% less runtime**. This compares `7361f1c` with the `06d83a2` optimization,
not with the original starter or another emulator. All **43,812 product-event
records**, semantic reports and exported bytes matched exactly in a separate
history comparison. There is no universal fastest-emulator or energy claim.
The final clean-source repetition used an automatic untimed history preflight:
median **1.008205 s to 0.869282 s**, **13.78% less runtime**, again eight
samples per binary with identical complete output history. Both repetitions and
all raw samples are preserved; this is not a confidence bound or a cross-host
prediction. Measured runs do not trace.

## Explicitly unfinished

- Complete CPU fetch/discarded-prefetch timing, interrupt-request sampling and
  all encoding/flag edge cases are not certified. Some phases remain provisional.
- SCI clock obligations, source-switch fractional phase, every power-retention
  matrix, and all local simultaneous-event rules still need closure.
- IIC2 and internal flash program/erase/verify remain unsupported. Some serial,
  ADC trigger and sensor interface/self-test/interrupt modes are incomplete.
- Timer W sub-state input timing/mux glitches, AEC prescaler/gate/module-stop
  ambiguities and comparator analog response are not physically characterized.
  Comparator response uses an explicit 15-microsecond maximum-derived witness.
- Sensor filter/calibration/axis skew, LCD scan/analog controls, supply/brownout,
  optical response and interrupted programming still contain documented models
  or unsupported boundaries. No HGSS physical-peer validation is claimed.
- Portable snapshot encoding, GUI/live-link integration, other host targets and
  the declared minimum Rust version have not been validated in this delivery.

[STATUS](STATUS.md) records the complete owner matrix;
[HANDOFF](HANDOFF.md) gives the next affected files and acceptance tests. This is
substantial implemented progress, not a relabeling of those remaining boundaries.
