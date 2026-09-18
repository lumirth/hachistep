# Emulator linking and host pacing

Source revisions below were inspected on 2026-09-18. HachiStep's decision is in
[DESIGN §13.5](../DESIGN.md#135-execution-pacing-and-external-connections).
These implementations show different hardware boundaries and host policies.

## SameBoy

The core exposes infrared output callbacks and an incoming infrared level for one Game
Boy, plus serial bit and external-clock APIs. The Cocoa frontend wires infrared between
instances and interleaves their execution using cycle offsets. Pairing is outside the
single instance signal API.
[Core API][sameboy-api], [frontend execution][sameboy-run].

`GB_run_frame` leaves pacing to its caller. The local-link frontend lets the primary
instance control pacing and routes speed changes across the pair. This shows that
software peers can share an accelerated pace; it does not require a physical-peer
adapter to support that policy.
[Frame API][sameboy-frame], [linked speed controls][sameboy-speed].

## mGBA and melonDS

mGBA's GBA SIO driver handles mode changes, register writes, and transfer
start/completion. Its local-link coordinator schedules timed serial work. The cable
hardware motivates this interface, which is not a generic infrared transport.
Fast-forward changes audio/video waiting and the target rate.
[SIO API][mgba-api], [local link][mgba-link], [pacing][mgba-speed].

melonDS supplies timestamped wireless frames and protocol-specific operations through
its platform interface. Its transport queues timestamped data; its frontend separately
controls normal, fast, and slow pacing with host clocks. That higher-level interface
reflects the wireless hardware it models.
[Platform interface][melon-api], [transport][melon-link], [pacing][melon-speed].

## PocketWalker

PocketWalker's Qt application owns TCP sockets, client/server connection setup,
reconnection, and byte accumulation. The hardware wrapper offers a transmit-byte
callback and receive-byte method. The adapter sends buffered bytes after a 5 ms
accumulation timeout, without optical-edge timestamps. Its receiver queues complete
bytes and moves the next one into the receive register when that register is free. [TCP
adapter][pocket-network], [application setup][pocket-setup],
[SCI implementation][pocket-sci].

That organization separates transport from emulation. The byte contract cannot preserve
pulse widths, malformed framing, or the arrival timing that determines hardware overrun.
HachiStep's arbitrary-firmware requirement therefore needs a signal interface. An
adapter can translate for a byte-oriented peer if a future consumer needs it, through
the ordinary hardware path.

PocketWalker's `Start` loop owns host pacing. Fast mode skips the sleep while retaining
the same SoC execution call. HachiStep places that pacing in the caller so its hardware
execution remains independent of host clocks.
[Execution loop][pocket-run].

## Timing responsibilities

Emulated time describes hardware effects; host pacing controls how much real elapsed
time their calculation takes. A speed multiplier changes the latter. Physical oscillator
settings remain separate device parameters.

A connection adapter must preserve relevant signal timing and deliver inputs before
execution crosses them. Timestamp retention cannot repair a physical response already
sent too late. Software-only environments can wait before advancing further; physical
peers keep running. These are adapter constraints, not reasons for the core to know
another emulator instance or to perform the firmware handshake.

Signal batching is compatible with this contract if it preserves the waveform and
arrives in time. The interface need not mandate one host message per edge.

[sameboy-api]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.h
[sameboy-run]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Cocoa/Document.m#L541-L583
[sameboy-frame]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Core/gb.c#L1190-L1248
[sameboy-speed]: https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/Cocoa/GBView.m#L325-L435
[mgba-api]: https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/include/mgba/gba/interface.h#L111-L130
[mgba-link]: https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/gba/sio/lockstep.c#L909-L1092
[mgba-speed]: https://github.com/mgba-emu/mgba/blob/25ca25612eb806ad3a70f3209ccd93890ea0c2c6/src/platform/qt/CoreController.cpp#L1304-L1340
[melon-api]: https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/Platform.h#L297-L307
[melon-link]: https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/net/LocalMP.cpp#L152-L334
[melon-speed]: https://github.com/melonDS-emu/melonDS/blob/906e9ebb27da8c6a715cd7abab4abfe8a8d29427/src/frontend/qt_sdl/EmuThread.cpp#L336-L404
[pocket-network]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/desktop/src/qt/network/qt_network_system.cpp
[pocket-setup]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/desktop/src/qt/emulator/emulator_context.cpp#L25-L36
[pocket-sci]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/core/soc/sci3/sci3.cpp#L63-L123
[pocket-run]: https://github.com/h4lfheart/pocketwalker/blob/2f3b4512a668e3b7c321f213c1c8d5344627e96e/core/pokewalker/pocketwalker.cpp#L32-L69
