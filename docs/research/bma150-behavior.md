# BMA150 behavior

The current sensor can be extended using documented behavior and useful vendor-code
evidence. Retail firmware exercises more than the normal ±2 g setting: it changes
protected register `0x1e`, repeatedly wakes and sleeps the sensor, and has a ±4 g,
25 Hz diagnostic. Those operations deserve working models, not unsupported-mode
errors.

Sources: Bosch **BST-BMA150-DS000-06, revision 1.6, 30 October 2008**
([manufacturer datasheet hosted by Digi-Key][ds]); Bosch-authored sensor API and
calibration code linked below; `lumirth/pw` commit
`6dc7bc09950078fa3fe0dffa4dae34e9549a99da`. This note concerns the current
`crates/hs-core/src/devices/bma150.rs`, inspected 2026-09-17.
Bosch's Linux driver identifies SMB380 as compatible with BMA150 apart from
packaging, which is why the older SMB380 API is relevant. ([Driver header][compatibility])

## Conversion, filtering, and reads

- **Conversion is sequential T, X, Y, Z**, with a nominal 3 kHz complete cycle;
  the digital bandwidth does not change register refresh frequency. The current
  simultaneous publication loses the documented distinction between reading X
  before and after its conversion. New-data IRQ occurs at Z publication only when
  all axes have been refreshed since their last reads. It remains latched until
  `reset_INT` or an acceleration-byte read, independently of `latch_INT`.
  ([§3.2.10, p.19][p19]; [§8.1, p.51][p51])
- The analogue path has a second-order 1.5 kHz low-pass; the digital path uses
  moving averages and initially runs at maximum bandwidth after wake. Filter
  lengths **64, 32, 16, 8, 4, 2, 1** for codes 0–6 are a well-supported inference
  from the stated successive halving of bandwidth. Preserve actual history and
  switch from the unaveraged startup output when filled. Exact integer rounding
  and sub-conversion phase spacing are model choices still requiring a hardware
  boundary measurement. Do not call the boxcar's exact recurrence proven solely
  by the nominal bandwidth table. Bosch's separate wake-current approximation
  also deserves checking against startup timing. ([§3.1.3, p.12][p12];
  [§7.3, pp.47–48][p47])
- With shadowing enabled, reading an axis LSB freezes its matching MSB until the
  MSB read. Freeze the already captured MSB across intervening conversions;
  disabling shadowing permits direct MSB-only reads. Either acceleration byte
  clears that axis's freshness flag. Serial prefetch must not acknowledge an
  unread *next* register when CS rises immediately after the preceding byte.
  This currently affects `read_register` calls while preparing subsequent output.
  ([§3.1.6, p.13][p13]; [§3.5.2–3, pp.23–24][p23])
- Temperature uses `°C = code / 2 − 30`: **20°C is code 100**, whereas the current
  code 80 represents 10°C. Acceleration is signed 10-bit, with 256/128/64 codes
  per g in the three ranges. ([§3.5.1, p.23][p23]; [table 1, p.5][p5])

## Registers, trim, and serial protocol

The default image is not all zero. Figure 1 gives `0x0b=0x03`, `0x0c=20`,
`0x0d=150`, `0x0e=160`, `0x0f=150`, `0x10=0`, `0x11=0`, `0x12=162`,
`0x13=13`, `0x14 & 0x1f = 0x0e` (±4 g, 1500 Hz), and `0x15=0x80`.
The high three bits of `0x14` are per-unit calibration, not disposable reserved
bits. Preserve the supplied nonvolatile image and distinguish these factory
defaults from firmware initialization. ([Register map and notes, pp.9–10][p9])

Each axis offset is the 10-bit value
`(reg[0x1a + axis] << 2) | (reg[0x16 + axis] >> 6)`; the other six bits in
`0x16 + axis` are gain trim. Bosch's calibration routine at ±2 g subtracts
`measured_error / 8` from the offset. This supports an offset increment producing
approximately **+31.25 mg**, or +8/+4/+2 output codes at ±2/4/8 g. Apply changes
relative to the modeled unit's calibrated factory trim; an arbitrary absolute
zero trim is not a calibrated sensor. Preserve gain bits on offset writes. The
gain field's transfer function was not established by these sources; do not
invent a multiplier from its bit width. ([Bosch driver offset access][offset];
[calibration calculation][calibration])

`pw` enables EE_W, reads `0x1e`, sets bit 7, writes it, and disables EE_W during
normal initialization. Implement that protected read/modify/write and retain its
value. This is direct evidence that treating the operation as an error is wrong;
it does not establish an invented name or analogue effect for bit 7.
([`AccelInit`, lines 68–94][pw-init])

Four-wire SPI is mode 3. **Only reads auto-increment**: with CS held low, a write
stream consists of successive address/data pairs. Return the parser to address
state after each written byte. Three-wire reads use the SDI net bidirectionally,
add a turnaround clock after the address, and use different output-edge timing;
four-wire SDO behavior cannot simply be reused. Writes use the same protocol in
both modes, permitting `pw` to force four-wire operation before reading identity.
([§4.1, pp.25–31, especially figures 6–9][p25]; [`AccelRead/Write`][pw-serial])

## Interrupts, wake-up, and self-test

Use filtered acceleration for interrupt criteria. Low/high-g qualification has
its own **1 ms counter ticks**, regardless of selected filter bandwidth. In
physical acceleration units with range magnitude `R`:

| Logic | Set criterion | Clear criterion |
| --- | --- | --- |
| Low-g | all axes `abs(a) ≤ threshold × R / 255` | an axis exceeds `(threshold + 32 × hysteresis) × R / 255` |
| High-g | any axis `abs(a) ≥ threshold × R / 255` | every axis is below `(threshold − 32 × hysteresis) × R / 255` |

Qualifying ticks increment the counter; trigger at `duration + 1`, then reset
the counter. On false criteria, counter mode 0 resets and modes 1/2/3 decrement
by that amount per tick. Hysteresis requires retained per-axis criterion state,
not a fresh comparison against one threshold on every tick. Nonlatched IRQ clears
when its combined criterion clears. ([§3.2.6–8, pp.15–16][p15])

Any-motion compares observations separated by three filter-output intervals,
`3 / (2 × bandwidth)`. Its threshold is four acceleration codes per register
step; its set **and clear** qualification requires 1/3/5/7 consecutive results.
Acquire four valid observations before evaluating the first difference: zeroed
history is not evidence of prior physical zero acceleration. Both any-motion
and alert require `enable_adv_INT`. Alert uses motion to shorten **working**
low/high-g qualification durations by 1 ms per ms; restore the configured values
after a low/high-g interrupt or when both reach zero. Do not rewrite their
configuration registers. ([§3.2.3–5, p.14][p14]; [§3.2.9, p.17][p17];
[§3.4.2, p.22][p22]; [p.48][p48])

Automatic wake pauses are 20/80/320/2560 ms. Wake, fill the selected filter,
collect enough observations for the enabled logic, then sleep when verification
finds no interrupt. A latched IRQ keeps the sensor awake until `reset_INT`;
nonlatched IRQ keeps it awake while the condition persists, with a minimum
330 µs assertion. Normal sleep retains configuration. Soft reset resets control,
status, qualification, and working image as power-on does; it cannot boot into
ordinary sleep. Nominal wake stabilization is 1 ms and cold startup 3 ms.
([§3.1.4–5, pp.12–13][p12]; [§3.3.6–7, p.21][p21];
[§7.2–3, pp.46–48][p46])

Self-test 1 substitutes zero acceleration at the ADC input and therefore runs
through the ordinary filter and low-g logic. Self-test 0 electrostatically
deflects the MEMS; completion clears its command and sets `st_result` for success.
A modeled healthy unit can complete it successfully; do not return Unsupported
or leave the command permanently busy. Its precise completion delay and
deflection under externally changing acceleration need a measured model, rather
than pretending self-test 1 and self-test 0 are identical. ([§3.3.4–5, p.21][p21];
[§3.4.1, p.22][p22])

There are genuine datasheet inconsistencies: p.22's `status_HG` bit number conflicts
with figure 1 and Bosch's header; use **bit 0**. The p.47 power table says 360 ms,
where the actual wake register definition says **320 ms**. Section 8.1 excludes
simultaneous new-data and other interrupt operation despite §3.2's broad OR
description; model documented individual modes without claiming that the mixed
configuration has been established. ([Bosch status definitions][status];
[pp.9, 13, 22, 47, 51][ds])

## Nonvolatile programming and power loss

EEPROM addresses `0x2b..0x3d` are **write-only**, even with EE_W enabled. EE_W
permits protected image reads `0x16..0x22` and writes through `0x3d`; otherwise
protected reads leave the output undriven. After **every completed EEPROM byte
write**, reload the complete working image, just as for `update_image`, reset,
and power-on. This can change SPI4 and other live settings. The current readable
EEPROM and completion without image reload contradict this behavior.
([§3, p.8][p8]; [§3.3.2–3, p.20][p20])

The current 28 ms programming duration is supported as a practical nominal
choice by Bosch's API delay constant; its offset-writing implementation waits
34 ms and the datasheet describes approximately 30 ms. These are operational
timing evidence, not evidence of atomic cells. ([Bosch API constant][ee-delay];
[offset EEPROM writer][ee-write]; [§3.4.5, p.22][p22])

Accept power loss during programming: cancel volatile work, resolve the affected
cell contents, then reset from those surviving contents. Carry the addressed byte,
old/target contents, and progress in the operation state. The cited sources do
not establish an erase/program bit order or an interruption-to-byte function.
A supplied partial-byte outcome as part of the physical power condition is a
concrete way to represent this without guaranteeing all-old/all-new atomicity;
it need not create a second execution path. Scope the outcome to the in-progress
byte rather than corrupting unrelated cells. The datasheet's prohibition on
sleep during programming describes valid firmware usage, not permission for the
emulator to reject a physical battery removal. ([§3.3.7, p.21][p21])

Useful behavioral checks are the real `pw` wake/read/sleep sequence, its
25 Hz diagnostic, reads straddling X/Z publication, shadowed LSB/MSB reads across
a conversion, and address/data/address/data writes under one CS assertion.
These exercise observable behavior without freezing private object layout.
([`CaptureSample`][pw-sample]; [`FactoryAccelDump`][pw-diagnostic])

[ds]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf
[p5]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=5
[p8]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=8
[p9]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=9
[p12]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=12
[p13]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=13
[p14]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=14
[p15]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=15
[p17]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=17
[p19]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=19
[p20]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=20
[p21]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=21
[p22]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=22
[p23]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=23
[p25]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=25
[p46]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=46
[p47]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=47
[p48]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=48
[p51]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=51
[offset]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L641-L674
[calibration]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L1360-L1435
[status]: https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.h#L399-L407
[ee-delay]: https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.h#L248
[ee-write]: https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.c#L254-L270
[compatibility]: https://kernel.googlesource.com/pub/scm/linux/kernel/git/tj/sched_ext/+/d023aa69c3b5f22a442fb67a37f17b04602eb43f/drivers/input/misc/bma150.c#8
[pw-init]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L68-L94
[pw-serial]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L18-L65
[pw-sample]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_main.c#L501-L534
[pw-diagnostic]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_selftest.c#L143-L167
