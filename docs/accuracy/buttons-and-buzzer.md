# Buttons and buzzer

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

Button inputs reach board pins and the same GPIO/interrupt/analog routing used by guest
accesses. Firmware performs polling and debounce. Timer/GPIO output determines piezo
drive, and timed buzzer events continue whether an application plays audio or mutes it.
The reusable audio converter produces PCM from these events.

## Limits and open questions

The input level history is supplied by the application. Mechanical contact bounce is
represented only when that history includes it. Audio conversion represents drive
timing, not a calibrated piezo, enclosure resonance or sound-pressure level. These
limits matter for physical acoustics and marginal contact behavior. They do not require
an interactive frontend as a prerequisite for reviewing the core's pin and timing rules.

## Button wiring and firmware use

The matching firmware reads CENTER on PB0, LEFT on PB2 and RIGHT on PB4, with a high
level representing a press. `InputInit` configures the wake interrupts, and `InputScan`
combines the sampled levels with a latched center event. Firmware owns hold durations,
debounce and the meaning of a press.
[Input routines](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_player_input.c#L12-L95).

The core supplies those levels to the existing pin resolver. Alternate ADC/comparator
uses of Port B still observe the connected physical input. In the nominal analog board
model, a pressed button supplies the rail voltage and a released button supplies zero;
an explicit analog fixture can override that voltage. The model does not add a contact
resistance, pull-network settling waveform or automatic bounce. These choices matter
if custom firmware measures the button electrically rather than treating it as a
stable digital input. [GPIO](bus-and-gpio.md) and [ADC](adc.md) describe the access rules.

## Piezo drive and audio

`BeepInit` configures P82/P83 and Timer W outputs B/C. Subsequent firmware controls the
waveform through Timer W. [Buzzer setup](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_buzzer.c#L49-L65).
The board resolves the two drive levels into positive, negative or neutral piezo drive.
Changing pin ownership, resetting the MCU or losing supply can change this drive through
the same board resolver. Equal terminal levels produce neutral drive in this digital
representation; it does not calculate piezo charge during a floating interval.

The [audio converter](../../crates/hs-core/src/audio.rs) resamples timed drive changes
into PCM. Playback rate, buffering and volume belong to the application. Muting playback
does not stop timer activity. Resampling history belongs to the converter and is
recreated after loading a machine state; the machine retains the hardware drive state.
The [presentation research](../research/emulator-presentation.md) explains this interface.
There is no measured mapping from its sample amplitude to physical sound pressure.

## Implementation and checks

[GPIO](../../crates/hs-core/src/mcu/gpio.rs) supplies button and output levels;
[board composition](../../crates/hs-core/src/machine.rs) emits changes to piezo drive.
[Timer W tests](../../crates/hs-core/tests/timer_w_modes.rs) check output generation,
[audio tests](../../crates/hs-core/tests/audio.rs) check conversion, and
[retail gameplay](../../crates/hs-core/tests/retail_play.rs) exercises button-driven
activity and sound through the embedding API. These check pin behavior, event timing
and software presentation. Physical contact bounce and acoustic calibration have the
limits described above.
