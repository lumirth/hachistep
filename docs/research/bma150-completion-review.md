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
itself; the later rev.1.7 datasheet mirror could not be retrieved. There is no new
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
