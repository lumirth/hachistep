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
  switch from the unaveraged startup output when filled. The implementation
  choices below make rounding, conversion spacing, and startup precise without
  mistaking those choices for extra specifications in the bandwidth table.
  Bosch's separate wake-current approximation describes an additional settling
  allowance; it is not a reason to double the inferred averaging window.
  ([§3.1.3, p.12][p12];
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

## Implementation choices from the diagrams and vendor APIs

The following rules make the digital model implementable. Explicit equations, byte
encodings, and protocol edges come from Bosch; the few finer boundaries not
specified there are identified once beside the chosen rule. These are usable
implementation decisions, with specific observations that can later refine them.
The source cache is `out/research/bma150.pdf` and `bma150.txt`; driver copies are
`bosch-bma150-driver.c`, `bosch-smb380-api.c`, and `atmel-bma150.c`.

### Serial transfers and shadow state

In four-wire mode, sample SDI on rising edges and change SDO on falling edges.
A write of `0x0c, 0x20, 0x0d, 0x02` under one CS assertion writes threshold 32
and duration 2; it does **not** put `0x0d` into register `0x0d` and `0x02` into
register `0x0e`. Each completed data byte returns the parser to control/address
state, so a subsequent control byte can also start a read without raising CS.
([Figures 5–7, pp.26–28][p26])

Three-wire writes have the same timing. For a read, counting rising edges from
the first address bit: edge 8 receives A0, edge 9 is the extra turnaround and
launches D7, and edges 10–17 sample D7–D0. On each sampling edge the sensor may
launch the following bit after its output delay. For a continued read, edge 17
therefore launches the next byte's D7; **there is no second turnaround clock**.
Drive SDI/SDA and leave SDO floating. On the fixed four-wire board this affects
the MCU's MOSI net; it does not reroute the data to MISO. The host must release
that shared net to receive three-wire data. ([Figures 8–9, pp.29–31][p29];
[`pw` serial initialization][pw-ssu])

Separate loading the serial shift byte from acknowledging its register read.
Use the first data sampling edge as the chosen read-side-effect boundary in
both modes: an address alone, or speculative preparation of the next byte,
must not clear freshness or release a shadow. Capture the byte and any matching
MSB needed for an LSB read at output launch; perform acknowledgement only if
the host clocks its first data bit. This also makes a three-wire read ending
at edge 17 acknowledge only its first byte. The exact acknowledgement edge is
not specified by the datasheet; the byte-level effects and avoidance of an
unclocked next-byte read are the required contract. ([§3.5.2–3, pp.23–24][p23])

Keep the latest converted axis separately from its held read pair. An LSB read
with shadowing enabled captures a pair; conversions continue but do not replace
that pair until its MSB is read. For repeated LSB reads before an MSB, retain
the same pair; that is the narrowest coherent interpretation of the documented
MSB-update block. Return the first LSB with the freshness it had before that
read cleared it. MSB-first reads with shadowing enabled are outside Bosch's
supported sequence; return the last serial MSB latch as the chosen behavior,
rather than silently promising a current coherent sample. With `shadow_dis=1`,
return the current MSB normally. ([§3.1.6 and §3.5.2][p23])

### Conversion and filter recurrence

Figures 3 and 29 draw equal T/X/Y/Z slots, and p.24 gives an 80 µs temperature
window. Use one rational **12 kHz phase clock**: after the chosen settled
acquisition epoch, T publishes at phase 1, X at 2, Y at 3, and Z at 4, then
repeat. Sample each axis input at its own conversion boundary. The ordering is
documented; equal spacing and the first phase after stabilization are the
compact timing inference. One oscillator phase also defines each 1 ms
qualification tick as 12 slots. A read after X publication but before Z clears
X's freshness and prevents that frame's new-data IRQ; a read before X
publication permits it. The IRQ test occurs after Z publishes and requires
all three freshness flags. ([Figure 3, p.19][p19]; [pp.24 and 51][p51])

For bandwidth code `b=0..6`, use `N=1<<(6-b)` raw samples per axis. Maintain
`sum' = sum + new_adc - oldest_adc` and publish `sum >> log2(N)` once N real
samples exist; before then publish the latest raw ADC value. A signed arithmetic
shift chooses floor rounding, consistent with discarding low bits in a
power-of-two hardware accumulator; the datasheet does not specify the rounding.
Keep the sum wide enough and clamp the ADC input to the signed 10-bit range
before averaging. This gives a precise moving average, not repeated division
of a previously rounded output. ([§3.1.3, p.12][p12])

Retain up to 64 actual samples. Recompute the selected running sum when bandwidth
changes, without resetting the oscillator or pretending earlier samples were
zero. Range and offset changes affect newly converted ADC codes; old history
flushes out over N samples. This agrees with the documented `1/(2*bandwidth)`
range-change settling time. Use the binary divider for time: code 0 has an
observation period of `64/3000 s`, not the rounded-label approximation of 20 ms.
([§3.1.2–3, pp.11–12][p11])

Bosch's approximate wake-current expression contains a **2N** acquisition
allowance followed by **3N** extra periods for any-motion. It also has a special
1500 Hz formula inconsistent with its own current table. Preserve the N-sample
filter above. For autonomous wake verification, choose the table-consistent
gate of 2N completed raw cycles after stabilization, then take the first
criterion observation; any-motion needs three further N-cycle intervals.
Ordinary register publication continues during that gate. This treats the
formula as a conservative acquisition/settling allowance, not a second filter
definition. The first inactive low/high check can sleep after that gate;
an active debouncer stays awake until it qualifies or becomes inactive.
([§7.3, pp.47–48][p47]; [ASF's reproduced formula][asf-wake])

### Offset calibration and the remaining gain field

Bosch explicitly calls the 10-bit trim **offset binary**, clips it to 0..1023,
and calibrates at ±2 g with `new_offset = old_offset - error/8`. The physical
input correction is therefore
`a_corrected_ug = a_ug + (offset - factory_offset) * 31_250`, followed by range
conversion and filtering. The factory baseline belongs to the modeled unit;
programming EEPROM or reloading its image must not move that baseline and erase
the calibration effect. This is stronger evidence than guessing from register
width. ([Offset access][offset]; [offset calculation][calibration];
[calibration range selection][calibration-setup])

The six gain bits, temperature trim, and register `0x14` bits 7..5 remain
per-unit calibration fields. Neither the Bosch BMA150/SMB380 API nor the
Atmel/Microchip driver examined supplies their numerical transfer function;
the datasheet explicitly omits temperature-offset trimming. Preserve these
fields and implement the supported offset path now. Do not turn `gain/32`,
`1+gain/64`, or an arbitrary step into a claimed Bosch formula. The focused
next evidence is a reversible gain-code change at fixed +1 g and −1 g:
half their output difference isolates sensitivity, while their mean isolates
offset. This remaining analogue calibration gap does not obstruct SPI,
conversion, interrupts, sleep, or firmware initialization. ([p.23][p23];
[vendor gain/offset register definitions][asf-registers])

### Qualification, wake, and self-test state

For a signed output code `c`, avoid fractional threshold rounding:

| Per-axis criterion | Set | Clear |
| --- | --- | --- |
| Low-g | `255*abs(c) <= 512*T` | `255*abs(c) > 512*(T+32*H)` |
| High-g | `255*abs(c) >= 512*T` | `255*abs(c) < 512*(T-32*H)` |

Retain the previous criterion inside its hysteresis band; combine low-g with
AND and high-g with OR. Use signed widened arithmetic for `T-32*H`, including
out-of-recommendation settings. On each 1 ms tick increment a qualifying
counter, otherwise reset it for mode 0 or saturating-subtract 1/2/3 for modes
1/2/3. Trigger and clear the counter at `working_duration+1`. Update criterion
states at axis publications so a nonlatched IRQ can deassert when its criterion
disappears, without waiting an extra millisecond. If publication and a timer
tick coincide, publish first, then evaluate that tick. This last tie ordering
is the chosen digital boundary. ([§3.2.7–8, pp.15–16][p15])

Sample the any-motion vector after Z every N raw cycles. Compare it with the
vector three such observations ago, not the immediately previous sample and
not an extra average of triples. Require four real observations before the
first difference. A component qualifies at an absolute difference of at least
`4*any_motion_thres` codes. OR the three components, count consecutive true
or false results, and assert/deassert after 1/3/5/7 results. Do not let reads
change this history. An alert-qualified motion event instead starts decrementing
the working low/high durations on their 1 ms ticks; restore configured
durations after a threshold interrupt or after both reach zero. Keep status,
source latches, and pin output separate. ([§3.2.9, pp.17–18][p17]; [p.22][p22])

Bosch's own mode setter writes `(wake_up,sleep)` as `(0,0)` for normal,
`(0,1)` for sleep, and **`(1,1)` for automatic wake**. ASF instead enables
automatic wake while sleep is clear, and Bosch permits boot into wake-up mode.
Reconcile these by keeping the automatic phase separate from the writable
control bits: `wake_up` enables periodic verification; `sleep=1` immediately
starts a sleeping interval, while clearing sleep begins active acquisition.
An active automatic phase returns to sleep after an inactive verification.
Clearing `wake_up` while awake restores continuous acquisition. A latched IRQ
prevents automatic sleep until reset; `reset_INT` restarts verification.
([Bosch mode setter][bosch-mode]; [ASF mode setter][asf-mode]; [§3.1.4][p12])

Use the nominal 1 ms wake stabilization and 3 ms cold startup. Reset while awake
uses the documented roughly 1.3 ms to available data; reset during sleep uses
the documented 30 ms bound as the chosen delay. Honor the 10 µs serial reset
quiet interval with undriven reads/ignored writes. Ordinary sleeping reads are
undriven; only the wake/reset control commands act. Keep the control image
through normal sleep and reset it through soft reset. ([pp.6, 21, 46][ds])

Self-test 1 holds the ADC-input override at zero while its bit is set; normal
filter history then drains to zero and low-g qualifies through its normal
threshold/duration logic. Self-test 0 must have separate command completion and
`st_result`: clear the previous result at start, then clear its command and
set the healthy-unit result at completion. A compact provisional completion
rule is one full T/X/Y/Z conversion cycle after the command, rounding completion
to Z. Keep the last complete output vector during that diagnostic cycle, then
resume ordinary acquisition. This models an internal measurement without
inventing a published deflection amplitude. Bosch gives no delay or amplitude;
ASF's immediate status read and manual command clear supply no reliable timing
measurement. This specific completion/publication rule is an inference to
replace with an observed trace, not a reason to reject the command.
([§3.3.4–5 and §3.4.1, pp.21–22][p21]; [ASF self-test wrapper][asf-selftest])

### Small external fixtures

These values follow the formulas above. Phase, filter rounding, wake-verification
gate, and self-test completion cases exercise the stated model choices; they
are not substitutes for independently recorded sensor traces.

| Setup/action | Expected observation |
| --- | --- |
| 20°C input | Temperature register `0x64` |
| Four-wire write stream `0c 20 0d 02` | Registers `0x0c=32`, `0x0d=2`; `0x0e` unchanged |
| Three-wire one-byte read | D7 sampled at rising edge 10, D0 at 17; SDO remains floating |
| Read X just before its phase, then await Z | New-data IRQ at that Z if Y/Z are fresh |
| Read X just after its phase, then await Z | No new-data IRQ until the following Z |
| Filled N=4 filter, zeros followed by repeated code 64 | Successive outputs `16,32,48,64` |
| Filled N=4 filter, one code-64 impulse | `16,16,16,16,0` |
| N=2 complete windows `[-1,0]` and `[0,1]` | Outputs `-1` and `0` under floor rounding |
| N=4 startup codes `4,8,12,16` | Outputs `4,8,12,10` |
| Baseline offset 512 changed to 513 at 0 g | Settled codes `+8/+4/+2` at ±2/4/8 g |
| Baseline 512 changed to 511, ±2 g | Code −8: fresh LSB `0x01`, MSB `0xfe` |
| Low-g `T=32,H=1` | Set at magnitude ≤64; clear at ≥129; retain at 65..128 |
| High-g `T=128,H=1` | Set at magnitude ≥258; clear at ≤192; retain at 193..257 |
| Duration 2, counter mode 2; criterion `T,T,F,T,T,T` on 1 ms ticks | Counter `1,2,0,1,2`, then trigger on tick 6 |
| Same sequence, counter mode 1 | Counter `1,2,1,2`, then trigger on tick 5 |
| Any-motion threshold 2, duration code 1, X observations `0,0,0,8,8,8,8,8,8` | First valid difference at index 3; IRQ sets at 5 and clears at 8 |
| N=64 automatic low/high verification, inactive input | First check 42⅔ ms after acquisition begins, then the selected sleep pause |

## Nonvolatile programming and power loss

EEPROM addresses `0x2b..0x3d` are **write-only**, even with EE_W enabled. EE_W
permits protected image reads `0x16..0x22` and writes through `0x3d`; otherwise
protected reads leave the output undriven. After **every completed EEPROM byte
write**, reload the complete working image, just as for `update_image`, reset,
and power-on. This can change SPI4 and other live settings. Preserve these
access restrictions and reloads through the parser and filter changes.
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
[p11]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=11
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
[p26]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=26
[p29]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=29
[p46]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=46
[p47]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=47
[p48]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=48
[p51]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=51
[offset]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L641-L674
[calibration]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L1360-L1435
[calibration-setup]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L1523-L1577
[bosch-mode]: https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c#L752-L776
[asf-registers]: https://github.com/avrxml/asf/blob/68cddb46ae5ebc24ef8287a8d4c61a6efa5e2848/common/services/sensors/drivers/bosch/bma150.h#L84-L91
[asf-wake]: https://github.com/avrxml/asf/blob/68cddb46ae5ebc24ef8287a8d4c61a6efa5e2848/common/services/sensors/drivers/bosch/bma150.c#L361-L378
[asf-mode]: https://github.com/avrxml/asf/blob/68cddb46ae5ebc24ef8287a8d4c61a6efa5e2848/common/services/sensors/drivers/bosch/bma150.c#L399-L431
[asf-selftest]: https://github.com/avrxml/asf/blob/68cddb46ae5ebc24ef8287a8d4c61a6efa5e2848/common/services/sensors/drivers/bosch/bma150.c#L567-L584
[status]: https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.h#L399-L407
[ee-delay]: https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.h#L248
[ee-write]: https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.c#L254-L270
[compatibility]: https://kernel.googlesource.com/pub/scm/linux/kernel/git/tj/sched_ext/+/d023aa69c3b5f22a442fb67a37f17b04602eb43f/drivers/input/misc/bma150.c#8
[pw-init]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L68-L94
[pw-serial]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L18-L65
[pw-ssu]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_ssu_init.c#L1-L18
[pw-sample]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_main.c#L501-L534
[pw-diagnostic]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_selftest.c#L143-L167
