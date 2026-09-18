# H8/38606F on-chip boot-mode implementation

Research, 2026-09-18. This is a functional contract for the manufacturer boot
program, using Renesas REJ09B0152-0300 rev. 3, the H8/38606 addition, and the
applicable correction. It complements [the flash owner](h8-flash-implementation.md)
and [SCI timing](h8-clock-and-sci.md). Printed hardware-manual pages are 34 below
their PDF page numbers. **Documented** protocol requirements and **chosen**
implementable boundaries are distinguished below; neither requires withholding
working boot support.

## Implementation

`machine/boot.rs` now implements this contract. Its pending accesses share the
CPU's existing bus-completion machinery; the SCI and flash owners are unchanged
by protocol intent. Original tests check the complete TXD waveform, full stop-bit
handoff, retained baud, reset during upload, native restoration within erasure
and the maximum materialized flash-state file. The independent hachiware cases
exercise blank/nonblank flash, all six blocks, odd upload length and invalid
length containment. Ordinary boot erasure is covered by the allocation gate.
The 100-state readiness interval comprises 25 setup-access states and a retained
75-state wait. Other private ROM instruction overhead remains unspecified.

## What is being supplied

The manufacturer's program receives a small **RAM programming-control program**,
not a complete replacement flash image. That uploaded program subsequently owns
whatever flash-programming protocol it implements. The target's public `pw`
source reconstructs retail user firmware, including its normal reset handler and
WDT-reset accounting; it does not supply the hidden manufacturer boot program.
Its reset handler therefore cannot substitute for boot mode.
([§6.3, pp. 104–107][boot], [pinned `pw` reset source][pw-reset])

Use one finite boot-service owner for the unavailable manufacturer program. Its
operations go through the existing MCU register/memory access authority, SCI,
flash, clocks and GPIO. It must not implement another UART, flash exposure model
or instruction interpreter. During this service the ordinary CPU does not fetch
the erased user reset vector. At handoff, that same CPU interpreter starts at
`FB80` and executes every uploaded instruction normally. No retail PC or payload
recognition is involved.

This is a functional replacement for unavailable ROM instructions: its serial
ordering, peripheral effects and physical delays are modeled, but its private
PC, scratch-register contents, RAM workspace and instruction-by-instruction
timing are not established. An authenticated boot-ROM image would allow this
policy to be superseded by ordinary execution; it is not grounds for adding a
second selectable fidelity path.

## Admission and reset

At **external reset release**, sample the resolved pins, stable for four states:

| TEST | NMI | E7_0 | Documented destination |
| --- | --- | --- | --- |
| 0 | 1 | Either | User reset vector |
| 0 | 0 | 1 | Manufacturer boot program |

A later NMI edge is not boot admission. The earlier correction explicitly
clarifies that reset release is the sampling point. Other strap combinations
have no documented user/boot destination: a compact chosen behavior is inactive
test state until another reset, rather than a host error or an invented factory
protocol. The fixed board must supply TEST/E7_0 levels; if E7_0 is represented as
high without wiring evidence, retain that as a local board inference. Retail
boot with NMI high alone cannot establish E7_0's level.
([§6.3, p. 104][boot], [correction, p. 1][correction])

Boot explicitly selects the **system oscillator**, even where normal startup
would select the on-chip oscillator. Apply this selection through the clock
owner without resetting raw oscillator phase. After valid reset release the
program is ready to measure RXD after approximately 100 states; choose 100
active system-clock states as the nominal functional delay. Missing/stopped
source clocks do not complete that obligation. TEST and NMI are specified to
remain unchanged throughout boot. ([§6.3.1, pp. 105–106][protocol])

External reset aborts the service and samples straps again at its next actual
release. WDT overflow also releases boot; the WDT owner already supplies the
documented 512-ROSC-cycle internal reset. **Chosen interpretation:** a WDT reset
clears the boot latch and takes user reset with WRST retained, while an external
RES release performs strap selection. This gives effect to the explicit WDT
exit rule without continuously re-entering boot from the unchanged low NMI.
External RES still held low prevents either destination from running.
([§6.3.1, p. 106][handoff], [§12.3.1, p. 208][watchdog])

The reset-default WDT is running. Stop it using its qualified register-write
sequence during boot initialization so an idle host does not cause periodic
resets. Re-enable it only around erase pulses as described below. This software
choice follows the indefinite host handshake and the recommended erase guard;
the actual boot program's watchdog instructions are unavailable.
([§12.2.1, pp. 203–204][wdt-control], [§6.4.2, p. 112][erase])

## Autobaud and actual pins

The host continuously sends `00` as ordinary, non-inverted **8N1 UART** on
P31/RXD3. A zero frame has nine low bit periods followed by one high stop period.
The target measures the low interval, calculates its baud setting, and returns
one `00`; the host then sends one `55`. This is not the optical IrDA pulse
encoding. SCI uses P32/TXD3; asynchronous operation needs no P30/SCK clock.
Reset should be released with RXD high.
([§6.3.1, p. 105][protocol], [§8.2.5, p. 127][pin-functions])

Recommended concrete initialization and measurement:

1. Keep P31 input and P32 high; clear SCI TE/RE and its interrupt enables,
   enable its module clock, disable IR and RX/TX inversion.
2. After the 100-state delay, wait for high followed by a fresh falling edge;
   if RXD was already low, do not measure a truncated frame. Measure from this
   falling edge to its rising edge in system-source phase coordinates.
3. With low duration `L` in system-clock periods, choose
   `q = round(L / 288)`, ties toward larger `q`, then `BRR = q - 1`.
   Select SMR=`00`, SEMR.ABCS=`0`, giving `baud = φ / (32q)`.
   Accept representable `1 <= q <= 256`; otherwise re-arm for a fresh pulse.
4. Wait at least `32q` source edges after setting BRR, enable SPC3 and TE/RE,
   and queue the single response `00` through TDR. The normal SCI owner supplies
   its documented initial frame of mark bits after TE is enabled.

The low-interval rounding and precise polling readiness are **chosen functional
rules**; the low-duration measurement, UART format, response and settling wait
are documented. A nonzero training pattern can miscalibrate the divider; do not
invent a second pre-SCI byte decoder to recognize the host's intentions.
([§14.3.8, p. 243][baud], [Fig. 14.4, p. 258][sci-init],
[Fig. 14.6, p. 260][sci-mark])

Table 6.3 qualifies 9600 baud at 8–10 MHz, 4800 at 4–10 MHz, and 2400 at
2–10 MHz. It is an operating envelope, not an opcode-like whitelist: try other
representable measured intervals without claiming the same hardware guarantee.
At 3.6864 MHz and 2400 baud, a bit is 1536 states; a zero's low interval is
13824 states; `q=48`, `BRR=2F`. At 8 MHz/9600 baud, rounding gives `q=26`,
`BRR=19`, actual 9615.3846 baud. ([Table 6.3, p. 108][baud-ranges])

Once enabled, consume only the existing SCI's RDR/status transitions. Training
zeros may still arrive while the acknowledgement is travelling; ignore valid
non-`55` bytes while awaiting `55`. Use normal TDR/TDRE, RDR/RDRF and error
handling, preserving actual bit phases and finite buffering. A host that sends
too far ahead can genuinely overrun the receiver. The concrete active setup is
SMR=`00`, SCR=`30`, SPCR=`D0`, IR disabled; it is not a decoded-byte shortcut.

## Erase, length, upload and handoff

Table 6.2 gives this ordering:

| Stage | Target action |
| --- | --- |
| Receive `55` | Check flash; if any data is written, erase every flash block. |
| Erase/check success | Send `AA`. |
| Erase failure | Send `FF`, then abort. |
| Receive length | Two bytes, **upper then lower**; echo each received byte. |
| Receive payload | Echo each byte and store it sequentially starting at `FB80`. |
| Receive the Nth byte | Finish its echo, send `AA`, then enter the RAM program. |

There is **no address field, checksum, escape convention, post-upload `55`, or
second-stage flash protocol** in this target's table. Its prose specifies lower
byte following upper byte; the matching two-byte sequence also appears in
Renesas's related H8/36109 manual. Do not import the different final handshake
or erase order of other H8 boot implementations.
([Table 6.2, p. 107][sequence], [H8/36109 Table 7.2, p. 113][related])

The upload aperture is exactly `FB80–FF7F`, inclusive: 1024 bytes. The target
addition expands RAM to `F780–FF7F` and flash to 48 KiB but does not amend this
boot aperture. Valid payloads may have odd lengths; do not pad or round them.
**Chosen out-of-range rule:** echo the two length bytes, then wait quietly for
reset when `N=0` or `N>0400`; do not write through peripheral addresses, jump to
unreceived code, invent another response byte, or return a host error. This is
a defined model boundary, not a claim that the unavailable ROM performs that
particular range check. ([§6.3.1, p. 105][protocol], [target addition][addition])

For erase, reuse the flash owner's array, verify latch and physical exposure:

- Scan all 48 KiB for non-`FF`. An entirely blank array skips erasure. Otherwise
  erase **all six blocks**, in chosen ascending order: four 1-KiB blocks, then
  `1000–7FFF` and `8000–BFFF`. Do not restrict erasure to the first dirty block.
- Use FLSHE, SWE, EBR1 and the documented Fig. 6.4 algorithm: SWE setup 1 µs;
  ESU setup 100 µs; E pulse 10 ms; E-off recovery 10 µs; ESU-off recovery
  10 µs; EV setup 20 µs. Verify aligned longwords via `FF` dummy writes with
  2 µs settling; clear EV and wait 4 µs before a retry. Limit each block to
  100 attempts. Clear SWE and wait 100 µs after completion/failure.
- Enable the actual WDT for each pulse and disable it before verification.
  A compact chosen guard uses φ/8192 and preload
  `256 - ceil(0.0198 * φ / 8192)` in the qualified clock range. At 3.6864 MHz
  this is TCWD=`F7`, giving nine input edges, nominally 20 ms. Preserve the
  shared divider phase; the first edge need not be one full period away.
- FLER or exhausted retries takes the documented `FF` failure path. Erase
  completion is established by verification, not an independent boot timer.

These are recommended boot-software choices around the documented flash
algorithm; the manufacturer ROM's block order and individual instructions are
not available. No new partial-programming model belongs here.
([§6.4.2/Fig. 6.4, pp. 112–113][erase], [target geometry][addition],
[WDT clocks, p. 207][wdt-clocks])

Before handoff, complete the final `AA` **including its stop bit**. SCI TEND
becomes true at stop-bit launch, so TEND alone is insufficient. Then clear
TE/RE, retain BRR, set PDR3.P32 and PCR3.PCR32, and select GPIO output by clearing
SPC3. Enter `FB80` without resetting peripherals or fetching a user vector.
Keep other general registers at the chosen reset/service values; they are
explicitly unspecified, including SP. Choose CCR.I set at entry; the uploaded
program initializes its stack and enables interrupts when ready. Boot service
does not admit interrupts, as required by §6.4.3; devices still accumulate their
ordinary flag state. Remove the service's admission inhibition on handoff.
([§6.3.1, p. 106][handoff], [§14.4.3, p. 259][sci-transmit],
[§8.2.5][pin-functions], [§6.4.3][erase])

## Retained state, interruption and failures

Retain only stage, source-clock obligation/measurement start, pending response,
length/cursor, blank/verify address, block and retry count, plus the existing
peripheral state. Drive the service from pin changes, SCI completion/status and
flash deadlines, not per-clock polling. Ordinary bus actions still occupy their
physical access states. Choose only the stated 100-state startup and required
bus/settling waits as nominal service overhead; do not fabricate a complete
private-ROM instruction schedule.

Silence has no documented timeout: wait indefinitely. A break waits for its
ending edge. A framing/parity/overrun error after SCI activation enters a quiet
failed state until reset; this is a chosen concrete response consistent with
the host's documented instruction to restart failed reception. Preserve the
SCI error flags rather than delivering a clean byte anyway. Only erase failure
has a documented `FF` reply.

On reset or supply loss, settle the old flash exposure and serial interval to
the physical boundary, then abort boot sequencing. Retain already changed flash
cells, and retain or lose received RAM bytes according to the existing MCU
reset/power domain. Do not roll either back to a pre-handshake image. A stopped
system clock freezes source-counted work; the existing flash/WDT power and clock
rules still apply. Snapshots include boot continuation and active raw-low
measurement, alongside SCI shift phase and flash progress.

## Original distinguishing cases

1. **Strap admission:** with TEST=0/E7_0=1, release RES with NMI=1 and later
   lower NMI: ordinary NMI behavior, no serial boot response. Release RES with
   NMI=0: no user-vector fetch, including when all flash bytes are `FF`.
2. **Measured autobaud:** at 3.6864 MHz present a complete low interval of
   13824 states after readiness; expect BRR=`2F`. Split execution or capture
   halfway through the low interval; divider selection and wire response stay
   identical. Begin low before readiness and end it afterwards; that truncated
   interval must not produce an acknowledgement.
3. **Small valid upload:** with blank flash, finish training/`55`, then send
   length `00 0A` and payload `7A 07 00 00 FF 70 01 80 40 FE`. This is
   `MOV.L #0000FF70,ER7; SLEEP; BRA -2`. The target byte stream is
   `00 AA 00 0A 7A 07 00 00 FF 70 01 80 40 FE AA`. The ordinary CPU sleeps
   with SP=`FF70`, next PC=`FB88`; BRR remains `2F`, TE/RE are clear and P32
   is high. The final stop bit must finish before GPIO takeover.
4. **Length/order:** send `04 00` and 1024 bytes, including `00/55/AA/FF`:
   they echo literally and the last byte lands at `FF7F`. `00 04` transfers
   four bytes, not 1024. `00 00` and `04 01` exercise the chosen quiet failure
   without upload writes or an invented `FF` response.
5. **Whole-array erase:** put one zero byte in EB5 and nonzero patterns in
   another block. After `55`, observe ordinary erase/verify operations and no
   success `AA` until all blocks verify erased. Repeating with wholly blank
   flash emits `AA` without a physical erase pulse.
6. **Interruption:** cut supply during an E pulse, reset after a payload prefix,
   and snapshot mid-echo. Flash retains the shared owner's partial exposure;
   reset abandons the pending upload; replay reproduces the remaining wire
   bits. No interrupted run emits a deferred success acknowledgement.

These fixtures establish observable protocol and integration behavior. They do
not assert an exact manufacturer boot-ROM instruction count or a calibrated
erase completion time beyond the flash owner's independently stated model.

[boot]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=138
[protocol]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=139
[handoff]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=140
[sequence]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=141
[baud-ranges]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=142
[erase]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=146
[wdt-control]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=237
[wdt-clocks]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=241
[watchdog]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=242
[baud]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=277
[sci-init]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=292
[sci-transmit]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=293
[sci-mark]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=294
[pin-functions]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=161
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[correction]: https://www.renesas.com/en/document/tcu/h838602-group-specification-changes
[related]: https://www.renesas.com/en/document/mah/h836109-group-users-manual-hardware
[pw-reset]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L194-L212
