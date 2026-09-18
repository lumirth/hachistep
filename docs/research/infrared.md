# Infrared board and firmware evidence

The core exposes timed incident and emitted infrared signals. The application owns their
transport, and firmware performs the link protocol. See
[DESIGN §13.5](../DESIGN.md#135-execution-pacing-and-external-connections).

## Pins and MCU timing

The board-facing polarity is supported by the actual firmware sequence: P30 high shuts
down the optical interface, P31 receives active-low pulses, and P32 drives transmission
high. `IrInitPins` selects PCR3=5/PDR3=1; `IrConfigure` selects receive inversion,
noninverted IrDA output, and 8N1; `IrHardwareStart` lowers P30; `IrFinish` raises it.
These are connected pins, so GPIO operation and SCI operation must continue through the
same board owner. [Firmware initialization and startup][pw-ir-start],
[firmware shutdown][pw-ir-finish].

At the canonical 3.6864 MHz system clock, BRR=0 gives 115200 baud and a 1.627604
microsecond nominal 3/16-bit transmit pulse. The H8 manual permits the MCU decoder to
recognize shorter input pulses; that statement describes the MCU input, not the
sensitivity of the external optical detector. Preserve the existing separation between
optical input and SCI decoding.
[Renesas SCI/IrDA chapter][h8-irda].

## Optical transceiver

The inspected board sources identify an unmarked SIR transceiver but no manufacturer or
part number. The matching firmware and captured 115200/8N1 traffic still constrain its
behavior. [Board investigation][board].

Manufacturer sources describe plausible mechanisms. The parts below provide comparison
values; choosing a Pokéwalker parameter requires board evidence or an explicit physical
inference.

| Mechanism | Manufacturer evidence | Consequence for HachiStep |
| --- | --- | --- |
| Receiver pulse shaping | ROHM RPM871-H14 gives a typical 2.3 microsecond RXD pulse for both shorter and longer incident pulses. | Incident light and the voltage on P31 need not have identical widths. An optical owner can retain a bounded pulse obligation. |
| TX protection | The same ROHM part terminates prolonged emission after typically 45 microseconds. | GPIO-held TX and stopped-clock stretched pulses can differ from indefinitely asserted light. The actual threshold needs board evidence or an explicit nominal inference. |
| Receiver recovery | RPM871-H14 specifies typical/max turnaround latency of 100/300 microseconds. | Recovery is physical elapsed time after transmission, distinct from an SCI clock count. |
| Shutdown and AC detection | Vishay's application note describes disabled reception in shutdown and a receiver unable to reproduce continuous DC illumination. | Shutdown, sustained light, and restart have physical state; a permanent active SCI input is not a general optical model. |
| Echo is part-specific | TFBS4711 explicitly mirrors transmit activity onto RXD and lists startup and recovery delays. | Its echo behavior must not be copied just because it uses SIR. |

Sources: [ROHM datasheet, electrical/optical table and timing diagram, pp. 4–5][rohm];
[Vishay circuit application note, pp. 7–9][vishay-note];
[TFBS4711 pin descriptions and receiver timing, pp. 2, 4][vishay-part]. The
Vishay documents inspected describe later revisions, including a 2022 product change;
they cannot establish a 2009 board's exact numerical behavior.

The firmware supplies an important constraint on self-echo. `SendPacket` waits for TEND,
waits for two Timer W counts, then drains a pending receive byte. However, `IrBegin`
sends its single CONNECT probe without that drain. A successfully decoded echo of this
probe would be treated as another walker by `IrProtocolTick`. Therefore the drain alone
does not justify unconditional self-echo. Suppression during local transmission is a
supported inference; echo-on, receiver blanking, and recovery need to be tested together
against this complete sequence. [Send/start/receive routines][pw-ir].

Useful physical observations include P30/P31/P32 plus emitted light during startup, a
local probe, sustained GPIO TX, and the first remote reply. Those distinguish multiple
plausible models with a short recording.

## Retail exchange constraints

The running firmware implements these operations. A diagnostic tool may decode the
resulting byte stream to explain its progress.

| Firmware action | Constraint on the modeled system |
| --- | --- |
| Start | Configure SCI, call `LowClockDelay` twice, lower P30, call it twice again, enable continuously running Timer W, clear SCI errors, drain old receive data. `LowClockDelay` executes its loop only when the firmware's low-power-clock flag is set; these calls do not establish a universal wall-time startup delay. |
| Probe/handshake | Send logical FC (wire byte 56 after XOR AA); respond with FA; acknowledge with F8. The common session token combines the two local tokens. |
| Packet reception | Poll RDR, record TCNT after each received byte, and treat silence as a frame boundary only when elapsed Timer W counts exceed 4. |
| Retry | When inactivity exceeds 0xC80 counts, retry with 0..15 times 0x60 counts of PRNG jitter, subject to the phase and retry limit. |
| Reply turnaround | `SendPacket` waits for TEND and two Timer W counts before draining receive data. TEND is already distinct from completion of the stop tail. |

These operations are in [the matching infrared source][pw-ir] and
[the conditional delay routine][pw-delay]. Timer W is selected to phiW;
at 32768 Hz one count is approximately 30.518 microseconds. Keep its actual counter
phase and strict comparisons instead of replacing this firmware with rounded host
timers. An exactly simultaneous pair of identical saved machines can make identical
retry choices; that is an intentional collision test, not the best initial success
workload.

## Firmware interoperability

The retail peer exchange requires registered EEPROM contents, a Pokémon, compatible
protocol fields, and distinct peer history. From the ordinary home screen, CENTER opens
the menu with CONNECT selected; another CENTER starts the attempt. A complete exchange
transfers status, records and PeerInfo, then PEER_START, and updates gifts, history and
the diary. Receiving a probe or valid header alone does not establish a complete
exchange. Firmware rejects a repeated encounter through its existing history checks.
[Home input][pw-home], [menu dispatch][pw-menu], [eligibility and transfer][pw-peer],
[completion][pw-complete], [persistent effects][pw-friend]. Nintendo instructs
users to select CONNECT on both units facing each other about 5 cm apart.
[Operations manual, section 6][nintendo].

Walker-to-walker and HGSS exchanges use different peers. Each needs its own
interoperability evidence through the ordinary signal interface. Connection setup, host
pacing and any test-environment scheduling remain outside the core.

[board]: https://dmitry.gr/?r=05.Projects&proj=28.%20pokewalker
[h8-irda]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=305
[rohm]: https://media.digikey.com/pdf/data%20sheets/rohm%20pdfs/rpm871-h14.pdf
[vishay-note]: https://www.vishay.com/docs/82610/irdatransceiver_referencelayoutscircuitdiagrams.pdf
[vishay-part]: https://www.vishay.com/docs/82633/tfbs4711.pdf
[nintendo]: https://csassets.nintendo.com/noaext/image/private/t_KA_PDF/Pokewalker_Tri?_a=BATCtdAA0
[pw-ir]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c
[pw-ir-start]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L65-L180
[pw-ir-finish]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L1150-L1160
[pw-delay]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/lib_common.c#L683-L699
[pw-home]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_home.c#L108-L155
[pw-menu]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_pictogram_menu.c#L46-L89
[pw-peer]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L461-L624
[pw-complete]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L64-L117
[pw-friend]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_friend.c
