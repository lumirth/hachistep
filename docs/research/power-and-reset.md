# Pokéwalker supply and reset behavior

The useful model is one board supply feeding devices with different retention,
startup, and reset behavior. Zero rail voltage stops the product; a low-battery
warning, a minimum rated operating voltage, and a reset threshold are different
things. This note applies the H8/38606 addition to REJ09B0152-0300, Bosch BMA150
Rev. 1.6, ST DS4192 Rev. 24, and Novatek NT7508 V1.0.

## Board evidence and independent chip control

The primary [board photographs][board-photos] and [board investigation][board]
show the coin-cell connection and BAT+/GND, VCI, and RES2B test pads. The latter
investigation identifies the EEPROM as M95512RP, but explicitly leaves RES2B
and VCI unidentified. Its claim that a CR2032 is nominally 3.3 V should not be
used as a voltage specification. Photographs establish component placement,
not continuity through every buried trace.

The matching firmware's [HardwareSetup][pw-setup] establishes P10/P11 as LCD
CS/D-C, P12 as EEPROM CS, and P90 as accelerometer CS. It does not toggle a
separate peripheral supply. [BatterySample][pw-battery] switches P84 around
ADC channel 7 readings; that is a measurement circuit, not evidence of a
peripheral power gate. Display and sensor sleep are entered through their
commands. These observations support a common battery-derived supply as the
default board model. No independently switchable supply to one of these three
chips has been established.

Therefore, do not add host-facing switches that power-cycle just the EEPROM,
sensor, or display on an otherwise unchanged board. MCU RES/watchdog reset,
BMA soft reset, and LCD software reset are separate existing mechanisms. A
shared-rail dip can affect chips differently because their internal domains
differ. Nor should the label RES2B alone be used to connect MCU reset to LCD
RESETB: retain the existing MCU-only meaning of `ResetPin` until an actual net
connection establishes more.

## Documented limits and sequencing

| Owner | Operating/retention conditions | Reset/startup consequence |
| --- | --- | --- |
| H8/38606 | VCC generally 1.8–3.6 V, subject to clock/mode limits; RAM retention rated from 1.5 V. | RES input, RC power-on circuit, and watchdog are distinct reset sources. |
| M95512-R | VCC 1.8–5.5 V. | Internal POR threshold is below VCC minimum; its numeric value/hysteresis are not specified here. |
| BMA150 | VDD 2.4–3.6 V; VDDIO 1.62–3.6 V, with VDDIO ≤ VDD. | Cold startup typically 3 ms; wake typically 1 ms, maximum 1.5 ms. |
| NT7508 | Logic VDD 1.65–3.6 V; analog VDDA/VCI 2.4–3.6 V. | Hold RESETB low during power establishment; analog drive and RAM/interface are different domains. |

References: Renesas [§21.2.1 and Table 21.2, pp. 386–397][h8], with the
[38606-specific additions][h8-add]; ST [Table 10, p. 25 and §5.1, p. 8][st];
Bosch [Table 1, pp. 5–6][bosch]; Novatek [pin descriptions, pp. 7–8][nt].
Absolute-maximum ratings are damage limits, not operating or reset thresholds.

**H8 reset is an RC process.** Section 19, pp. 369–370, describes an external
RES capacitor charged through the internal pull-up. Reset release follows its
threshold crossing and eight φ clocks. Table 21.10, p. 408 gives RES threshold
0.7/0.8/0.9 × VCC minimum/typical/maximum and pull-up 60 kΩ minimum, 100 kΩ
typical. Reliable cold retrigger requires VCC below 100 mV and discharge of the
RES capacitor. A dip recovering above that condition need not reset the MCU.
There is no separate brownout-reset source in Table 3.2. [Renesas §§3.2, 19][h8]

Preserve the RTC across RES and watchdog reset. Its cold-power register values
are unspecified, not a hardware-guaranteed all-zero reset. RAM's 1.5 V retention
rating likewise does not promise immediate erasure below 1.5 V.
[Renesas §§11.4.1, 11.6.2 and Table 21.2][h8]

Oscillator readiness is separate from reset release: Table 21.3, p. 400 gives
system-crystal stabilization 300 µs typical/800 µs maximum at 2.7–3.6 V;
the subclock crystal can require 2 s at 2.2–3.6 V. These are conditions and
bounds, not one universal startup delay. [Renesas Table 21.3][h8]

**EEPROM POR establishes a new serial session.** It clears WEL/WIP, releases
HOLD, preserves persistent cells/status, and requires CS to have been high
before a new falling edge selects the chip. Stable valid supply is required
through the entire internal write. The power-down prescription to finish writes
is an application guarantee, not an instruction for an emulator to reject
battery removal. Use the interrupted-cell model in
[lcd-and-eeprom.md](lcd-and-eeprom.md). ST specifies no additional fixed delay
after valid stable power. Older applicable [Rev. 18 §5.1.3, p. 13][st-old]
also limits the rising slope to 1 V/µs; Rev. 24 omits that sentence, so it should
not become a silently invented POR timer. [ST §§5.1, 7.1][st]

**Sensor startup is not sleep wake-up.** Cold/soft reset reloads its EEPROM
image and boots normal or wake-up mode, never sleep. Soft reset forbids serial
traffic for 10 µs; that is shorter than sample readiness. Waking then resetting
typically takes 1 ms + 1.3 ms at maximum bandwidth; soft reset while asleep
can take up to 30 ms. Preserve those different origins of startup.
[Bosch §§3.3.6–3.3.7, p. 21 and §7.2, p. 46][bosch]

For the LCD, RESETB low width is at least 10 µs at 2.7–3.6 V or 20 µs at
1.65–2.7 V; reset completion is at most 1 µs. Its initialization flow separately
waits for supply/drive stabilization. Power save retains configuration/RAM and
stops drive; it is not battery removal. [Novatek pp. 45, 52–54, 62][nt]

## Concrete implementation guidance

1. Give supply events one physical meaning: voltage at the board supply rail.
   An explicit `power_off` can represent a hard rail collapse; battery contact
   removal with capacitor hold-up can instead be supplied as a voltage
   trajectory. Do not mix those meanings or assume unplugging makes every
   capacitor instantly empty.
2. At a rail-collapse boundary, settle old clocks and programming progress up
   to that instant, resolve the partial persistent contents, cancel future
   powered activity, and remove active output drive. At 0 V with no alternate
   source, no CPU, sensor conversion, EEPROM programming, or RTC progression
   continues. A zero-voltage input must reach this path rather than merely
   changing the ADC scale; that improves the current API's contrary convention.
3. On restoration, start device readiness independently. EEPROM selection
   needs a fresh CS edge; BMA needs cold startup; MCU execution waits for reset
   release and oscillator readiness. Resolve GPIO reset states and resulting
   CS edges through the normal board nets. MCU-only reset must not cancel an
   external write already accepted by the EEPROM.
4. Model RES-capacitor state and reset-release counting rather than resetting
   on every voltage decrease. Use the documented typical RES ratio/pull-up and
   a single canonical board capacitance/startup realization. Keep any inferred
   capacitance in the physical model, not a firmware-specific delay or public
   accuracy option. Preserve capacitor charge through short supply dips and
   save states.
5. For an initial deterministic undervoltage realization, a device's minimum
   operating limit can serve as a conservative **functional availability**
   boundary. This is a model choice, not a measured POR threshold. Suspend the
   unavailable function, handle interrupted programming, and retain unaffected
   latches; do not synthesize a clean reset at that crossing. Separate loss of
   LCD analog drive from its lower-voltage digital state. Centralize these
   boundaries so later physical evidence changes parameters rather than the
   execution architecture. No undocumented hysteresis pair needs to be invented.

Finally, let firmware implement battery policy. `pw` averages eight ADC samples
and compares them with a checksummed EEPROM calibration. Boot uses 19/20 of
the recorded threshold and waits while low; normal warnings use 20/20. It also
counts watchdog resets separately. Substituting a hard-coded emulator battery
warning or generic clean reboot would bypass observable firmware behavior.
[Battery routines][pw-battery], [startup lines 193–242][pw-startup]

Useful distinguishing cases are a complete 0 V interval, a short dip preserving
RES charge, valid-voltage MCU reset during EEPROM programming, cold boot with
CS already low, BMA reset during sleep, and save-state restoration during reset
release/startup. These exercise physical transitions without pinning tests to
private object layout.

[board]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/doc/Board.md
[board-photos]: https://github.com/mamba2410/reverse-pokewalker/tree/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics
[h8]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[h8-add]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[st]: https://www.st.com/resource/en/datasheet/m95512-r.pdf
[st-old]: https://docs.rs-online.com/4548/0900766b81171451.pdf
[bosch]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf
[nt]: https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf
[pw-setup]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_ssu_init.c#L4-L18
[pw-battery]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_battery.c#L47-L119
[pw-startup]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L193-L242
