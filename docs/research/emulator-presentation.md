# Emulator output and frontend presentation

Research checked 2026-09-16. SameBoy and jgenesis provide concrete examples of the
boundary between emulated display/audio hardware and host presentation. They support
keeping controller behavior in the core while deferring decisions about reusable
presentation helpers until a frontend needs them. They do not establish one mandatory
output format or location for every filter.

## Observed practice

SameBoy produces pixels and sampled audio inside its reusable core. Its display API
accepts an output pixel buffer, an RGB encoding callback, and a vertical-blank callback.
Palette and color-correction controls also belong to that API. Its audio API delivers
signed 16-bit stereo samples at a configured sample rate, with selectable high-pass
filtering. The frontend therefore does not have to emulate the Game Boy PPU or APU, and
audio consumers are not required to reconstruct sound from raw electrical transitions.
([Display
API](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/display.h#L98-L105),
[sample representation](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/apu.h#L29-L44),
[audio configuration](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/apu.h#L220-L227))

SameBoy's SDL frontend combines current and previous pixel buffers when frame blending
is selected, renders through a chosen shader, and queues samples to an SDL audio device.
This is a practical division of responsibilities, despite some appearance and audio
conversion operations living in the core itself. There is no requirement that every
operation affecting appearance be a frontend operation. ([Frame
blending](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/SDL/main.c#L755-L766),
[shader selection](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/SDL/main.c#L1598-L1600),
[audio device integration](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/SDL/audio/sdl.c))

jgenesis separates Rust emulation backends from rendering and device services. Its Game
Boy backend obtains the completed PPU frame, converts it into an RGBA buffer, and calls
a supplied `Renderer`. It drains audio into an `AudioOutput` whose interface accepts
stereo samples. These interfaces describe usable output; they do not ask the caller to
interpret LCD controller writes. ([Game Boy output
delivery](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/src/api.rs#L252-L263),
[consumer interfaces](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/common/jgenesis-common/src/frontend.rs))

Its shared GPU renderer applies optional LCD color correction and frame blending. Its
Game Boy audio backend removes DC offset and resamples before delivering samples, while
the native frontend supplies playback buffering, gain, and SDL device integration. This
demonstrates that shared presentation code can become useful without making resampling
or filtering universally frontend-owned. ([Renderer
processing](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/frontend/jgenesis-renderer/src/renderer.rs#L186-L280),
[backend audio conversion](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/backend/gb-core/src/audio.rs#L63-L105),
[native audio integration](https://github.com/jsgroth/jgenesis/blob/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8/frontend/jgenesis-native-driver/src/mainloop/audio.rs#L226-L296))

## Application to HachiStep

The [design](../DESIGN.md#127-frontend-integration) assigns these responsibilities. Keep the LCD
controller's commands, RAM, addressing, control state, and relevant timing in the core.
A frontend consumes the resulting display state and makes it visible; it should not
implement another controller parser. LCD tint, a drawn pixel grid, window scaling, and
host refresh scheduling are presentation choices. The existing [display
contract](../DESIGN.md#125-display-output) already distinguishes controller
interpretation from requested pixel conversion.

Similarly, timer and pin behavior driving the buzzer remain part of hardware execution.
Muting playback must not stop their progression or erase effects on other emulated
hardware. Converting their output into audible samples and feeding an audio device is a
separate responsibility. The mature examples above permit sampled audio APIs; they do
not justify requiring every emulator consumer to handle raw waveforms. HachiStep's
compact output descriptions should serve its actual performance and fidelity
requirements.

While building the first frontend, decide whether display conversion, audio conversion,
or appearance effects deserve shared helpers. Extract useful common code when that work
makes the boundary concrete. Do not introduce a presentation framework or detailed
physical panel/piezo simulation as a prerequisite for a working, accurate core. Any
later physical model needs a specific purpose and evidence; it is not implied by the
word "emulator."
