# BMA150 completion review

Reviewed 2026-09-18 against the current owner, `bma150-behavior.md`, Bosch
BST-BMA150-DS000-06 rev.1.6, Bosch-authored driver/calibration code, and matching
`pw` at `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`. Source review only; no builds or
tests were run. Prior HachiStep implementations were not consulted.

## Two corrections to make first

**Do not stretch a read-acknowledged new-data interrupt.**
`bma150/control.rs:55` installs `irq_hold` for every rising IRQ in automatic wake
mode. `bma150.rs::acknowledge_read` clears `data_ready`, but that hold keeps INT
asserted until 330 µs expires. Bosch defines new-data as latched independently of
`latch_INT` and acknowledges it on any acceleration-byte read; the automatic-mode
minimum width applies to nonlatched interrupts. ([§3.1.4–5, p.13][ds13];
[§3.2.10, p.19][ds19])

Retain the existing separate `data_ready`, criterion latches, and `irq_hold`.
Create a hold only for a rising **nonlatched internal criterion output**, never
merely because the combined pin rose. A data read clears only data-ready; it must
not clear an independently justified criterion hold. `reset_INT` clears both.
This needs no new state if the existing hold is restricted to that meaning.

**Image reload must preserve an outstanding shadowed pair.**
`bma150.rs:321` unconditionally discards `shadows` in `copy_image`. Consequently,
an LSB read followed by an identical `update_image` and then an MSB read returns
`last_msb` instead of the held pair. Bosch holds the MSB until its read;
`update_image` reloads configuration/trim, not acceleration data.
([§3.1.6, p.13][ds13]; [§3.3.2, p.20][ds20]; [§3.5.2, p.23][ds23])

Chosen inference: image replacement preserves published bytes, held pairs,
freshness, and conversion phase; release pairs only if the resulting
`shadow_dis` disables shadowing. Cold/soft reset still clears them explicitly.
Apply this rule both to explicit image update and the download after an EEPROM
write. Future range/trim changes affect conversions without rewriting an already
captured read pair.

## Keep the existing offset and digital-filter model

The driver's offset getter assembles two low bits and eight high bits; its
calibrator subtracts `error/8` at ±2 g. The current correction follows that
evidence. ([Offset access][offset]; [calibrator][cal])

```text
O_i = (reg[0x1a+i] << 2) | (reg[0x16+i] >> 6)
a_i = physical_input_ug + (O_i - O_factory_i) * 31_250
c_i = clamp(trunc(a_i * 512 / (R * 1_000_000)), -512, 511)
```

Keep `O_factory=512` for the current canonical unit. It is the immutable physical
baseline, not the latest EEPROM value. An EEPROM save programmed by firmware must
not redefine it. Split offset writes take effect independently at their bus
completion; the vendor API itself writes the low and high parts separately.
Consequently, an intervening conversion may see their temporary combined value.
Preserve the six gain bits when updating the offset.

For `N=1<<(6-bandwidth)`, retain actual ADC-code history and
`sum += new-old`; publish `sum >> log2(N)` after fill, or the newest code during
startup. Range changes affect new codes only. With a filled N=4 filter at +1 g,
changing ±2 g to ±4 g produces `224,192,160,128` from an initial code 256. Do not
rescale old samples, clear the history, or publish early. The vendor range setter
also documents the settling interval. ([ASF range setter][range])

`pw` preserves register `0x14` calibration bits, selects ±2 g/1500 Hz normally,
and uses ±4 g/25 Hz in its factory diagnostic. Retain those as separate useful
workloads. ([Initialization][pw-init]; [diagnostic][pw-test])

The Bosch register definitions establish gain fields and temperature trim, but
the inspected APIs provide no numerical gain transfer. Searches for ANA016 and
its newer identifier BST-MAS-AN014-01 found references, not the application note
itself; the initial pass could not retrieve the later rev.1.7 datasheet (see
the source follow-up below). There is no new
evidence here for `gain/32`, `1+gain/64`, or a temperature-offset step. Keep those
bytes writable/retained and the specific missing analogue effect recorded; do
not disguise an arbitrary multiplier as calibration completion.
([Bosch gain-field definitions][gain])

## The next substantive signal-path improvement

`sample_phase` currently converts the latest physical input directly into an ADC
code. Bosch specifies a second-order analogue low-pass before digital averaging.
([§3.1.3, p.12][ds12]) A pulse wholly between two axis conversions presently
vanishes, and high-frequency content aliases without analogue attenuation.

A compact **proposed physical inference**, not a recovered Bosch transfer
function, is a unity-DC, maximally flat two-pole filter at 1500 Hz. Per axis retain
output `y`, derivative `v`, and the last physical-update instant. For constant
input `u` over elapsed `dt`, set `k=2π·1500/√2`, `e=exp(-k·dt)`,
`c=cos(k·dt)`, `s=sin(k·dt)`, then advance from the old values together:

```text
y' = u + e * ((c+s)*(y-u) + (s/k)*v)
v' = e * ((c-s)*v - 2*k*s*(y-u))
```

Settle the old input before each physical trajectory change and at that axis's
conversion aperture; sample `y`, then apply trim/range, saturate the ADC, and feed
the existing digital filter. This deliberately keeps the documented digital
recurrence unchanged. Use deterministic fixed-point coefficients with specified
rounding, a precomputed ordinary conversion interval, and widened products.
Arbitrary host run ends, inspection and capture must not integrate or round this
state. Save both analogue state variables and their timing reference; published
ADC history cannot reconstruct a pulse still decaying between conversions.

The Butterworth damping and trim placement are the chosen inference. Do not claim
the cutoff alone proves that damping. This is nevertheless a concrete improvement
over an absent analogue stage and keeps physical behavior in one execution path.

## Discriminating fixtures

1. Automatic wake with only new-data enabled: acknowledge the first IRQ with an
   acceleration read within 100 µs. INT falls at acknowledgement. Repeat with a
   nonlatched internal criterion to preserve its 330 µs minimum instead.
2. Read X LSB, reload an identical image across several conversions, then read
   X MSB. The original pair survives; a following pair sees current data.
3. Fill N=4 at +1 g, switch range between X and Y apertures, and observe each axis
   independently. Check the mixed old/new-code sequence and ordinary freshness,
   including a snapshot during the transition.
4. At 0 g, change offset 512→513 while preserving gain bits. Settled codes are
   +8/+4/+2 for ±2/4/8 g. Program/reload that offset and confirm its effect remains.
5. Place a short acceleration pulse between two X apertures. The proposed analogue
   state leaves a decaying response; shifting the pulse changes that response.
   Compare partitioned execution and capture/restore, then use a separate
   frequency sweep to discriminate damping rather than fitting to this pulse.

## Analogue stage across power states

Follow-up source review, 2026-09-18. Bosch specifies about 200 µA in normal
operation versus 1 µA in sleep, fresh acquisition on wake, 1 ms nominal wake
readiness (1.5 ms maximum), and 3 ms cold readiness. Sleep retains control/image
registers; soft reset has power-on-reset register effects, with about 1.3 ms to
available data when awake or up to 30 ms when reset asleep. These facts support
an inactive measurement path during sleep, but do not specify capacitor leakage
or the two-pole stage's initial voltage. [Table 1, pp.5–6][ds5], [§3.1.4][ds12],
[§§3.3.6–7][ds21], [§7.2, p.46][ds46]

Choose this compact realization for the proposed filter. Let `d = v/k`, so both
saved state values have acceleration units. The physical input continues to
change while the electronic stage is inactive.

| Transition | Selected analogue behavior |
| --- | --- |
| Cold construction/full power return | Initialize `y=d=0` at the valid-supply instant, retain the current physical input, and integrate through the existing 3 ms startup. This is a canonical initial voltage, not a Bosch zero-state guarantee. |
| Retained nonzero undervoltage dip | Settle the old powered interval at supply loss, then freeze `y,d`. Changes of acceleration update the physical input only. At valid return, rebase the integration time without integrating the absent interval; resume through the existing 1 ms reacquisition. |
| Ordinary or automatic sleep/wake | Settle at sleep entry and freeze. On wake retain `y,d`, rebase time to wake, and resume through the entire existing 1 ms interval. Do not track sleep-time motion through a nominally powered filter or force its output to the new input at wake. |
| Soft reset | Choose the same zero analogue state as cold reset at the command boundary; integrate through the existing 1.3/30 ms readiness. Keep the documented register, quiet-time, and digital-filter reset effects distinct. |

Freezing warm state is an explicit retained-charge inference. It avoids adding
an unsupported leakage constant or a sleep-time acquisition path; it does not
claim the unpowered MEMS structure stops moving. Cold and soft-reset zeroing are
also selected initial conditions. The existing readiness intervals allow the
new input to settle before conversion without inventing another delay.

The analogue stage is active whenever supply is valid and the sensor is awake,
**including while `wake_deadline` is pending**. Current `acquisition_delay`
places X/Y/Z at 5/6, 11/12, and 1 ms after an ordinary wake. Starting analogue
evolution only when the conversion clock starts at 2/3 ms would wrongly reduce
the first X settling interval to 1/6 ms. For a constant step from a previously
settled input, the proposed filter's remaining errors at those apertures are
0.03056%, 0.18039%, and 0.16586% of the step. Those are independently evaluated
model consequences, not measured Bosch responses. [Current acquisition and
reset transitions][control-current], [conversion apertures][bma-current]

Physical input changes must carry their timestamp: integrate the old input
before replacing it. Integrate at power/sleep/reset boundaries and real axis
apertures, never at arbitrary run ends, inspections, or capture. A pulse wholly
inside sleep that returns to the old input leaves no acquired pulse response;
a pulse during wake settling does affect the first conversion. Do not snap to
steady state at readiness. Even a tiny residual can change a truncated ADC code
at a threshold: cold +1 g leaves only 0.002749 µg of error at 3 ms under the
zero-state choice. Specify fixed-point and final ADC rounding separately, and
use inputs away from quantizer boundaries for filter/readiness fixtures.

### Numeric bounds for the proposed state

For `|u| <= U = 1_000_000_000 µg`, the ideal filter's bounded-input gains are
`||h_y||1 = coth(π/2) = 1.090331411` and
`||h_d||1 = 2√2 exp(-π/4)/(1-exp(-π)) = 1.347832909`.
Thus overshoot is legitimate; do not clamp the analogue state to the selected
±2/4/8 g ADC range. Apply trim/range and ADC saturation afterwards.

A simple conservative validation domain follows directly from the proposed
ODE. With `q=y+d` and `r²=y²+q²`, it gives
`dr/dt <= -k*r + 2*k*U`. A radius **4U** therefore contains nominal cold/steady
trajectories with ample rounding margin and is invariant under the exact
equations, including frozen intervals. Q16 acceleration in signed `i64` and
signed `i128` coefficient products are sufficient: `|y|,|q|<=4U`, hence
`|d|<=4√2U`. For saved-state validation, widen before forming `q`, reject either
component outside the radius before squaring, then check the squared radius
in `i128`. Keep coefficient rounding deterministic and check that it preserves
stability; this bound must not become a runtime signal clamp.

Also validate input bounds and `analogue_at <= machine.now`; a frozen
unpowered stage cannot have advanced past its supply-loss instant. Check long
elapsed intervals before multiplying full-width timestamps by fixed-point
rates. At 10 ms the maximum allowed residual is far below one Q16 unit, so
the rounded steady limit can be used without evaluating an enormous argument.
These bounds derive from the chosen transfer equations above, not a claimed
Bosch numeric implementation.

## Calibration source follow-up

The follow-up recovered Bosch BST-BMA150-DS000-07 rev.1.7 (29 June 2010) from
a different mirror. It still supplies no numerical gain transfer, and §3.5.1,
p.24 expressly leaves temperature-offset trimming undescribed. Its history
records the ANA016 reference changing to BST-MAS-AN014-01, not an added trim
specification. The application note itself was not recovered. Keep the existing
offset model and the recorded gain/temperature limitations. ([Rev.1.7 temperature
section][rev17-temp], [revision history][rev17-history])

Bosch's accessible **BST-MAS-AN030-01 rev.1.1** (October 2019), also titled
*Inline calibration*, does not fill that gap. Its applicability list omits all
three BMA150 reference codes; §6 uses a different register layout, with a
calibration trigger at 36 and correction registers at 38–3A. It describes offset
correction, not the BMA150's six-bit gain fields. Do not import those register
effects or a sensitivity scale into this owner. No additional numerical trim
behavior is justified by these newly checked sources. ([Applicability, pp.1/3;
implementation, pp.5–6][inline-2019])

[rev17-temp]: https://datasheet.datasheetarchive.com/originals/library/Datasheets-EDS4/DSAEDA00066689.pdf#page=24
[rev17-history]: https://datasheet.datasheetarchive.com/originals/library/Datasheets-EDS4/DSAEDA00066689.pdf#page=58
[inline-2019]: https://www.bosch-sensortec.com/media/boschsensortec/downloads/application_notes_1/bst-mas-an030.pdf
[ds5]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=5
[ds21]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=21
[ds46]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=46
[control-current]: ../../crates/hs-core/src/devices/bma150/control.rs
[bma-current]: ../../crates/hs-core/src/devices/bma150.rs
[ds12]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=12
[ds13]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=13
[ds19]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=19
[ds20]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=20
[ds23]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=23
[offset]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L641-L674
[cal]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L1360-L1435
[gain]: https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.h#L282-L289
[range]: https://github.com/avrxml/asf/blob/68cddb46ae5ebc24ef8287a8d4c61a6efa5e2848/common/services/sensors/drivers/bosch/bma150.c#L440-L458
[pw-init]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L68-L94
[pw-test]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_selftest.c#L146-L169
