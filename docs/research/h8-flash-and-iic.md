# H8/38606F flash and IIC2

Research against the current starter, 2026-09-16. This note separates documented
behavior from proposed implementation choices; it does not change the design.

## Sources and applicability

The target is HD64F38606: use the H8/38602R manual **REJ09B0152-0300,
revision 3.00**, with **TN-H8*-A414A/E**. The addition changes memory size,
erase-block geometry and EBR1, while retaining the other peripheral specifications.
It specifies 48 KiB flash at `0000–BFFF` and 2 KiB RAM at `F780–FF7F`.
([Target addition, pp. 1–4][addition])

Manual page references below are printed pages; add 34 for PDF page numbers.
The addition has one cover page. Relevant late IIC updates are
**TN-MC*-A022A/E** and **TN-MC*-A023A/E**; both name H8/38602R. Applying them to
the unchanged H8/38606 IIC2 is the appropriate implementation inference.

## Internal flash: registers and geometry

All these registers use an 8-bit bus and two access states. Reset values are zero.
([Manual §6.2, pp. 101–104; §20.1, p. 372][manual-registers])

| Address | Register | Bits and access |
| --- | --- | --- |
| `F020` | FLMCR1 | `40 SWE`, `20 ESU`, `10 PSU`, `08 EV`, `04 PV`, `02 E`, `01 P`; bit 7 reads zero |
| `F021` | FLMCR2 | Read-only `80 FLER`; other bits zero |
| `F022` | FLPWCR | Read/write `80 PDWND`; other bits zero |
| `F023` | EBR1 | EB0–EB5 select erase blocks; functional mask `3F` |
| `F02B` | FENR | Read/write `80 FLSHE`; other bits zero |

`FLSHE` gates CPU access to the four other control registers. `SWE=0` prevents
setting other FLMCR1 bits and initializes EBR1. Selecting multiple EBR1 bits
clears its selection. EBR1 bits 7–6 are described as readable/writable reserved
bits with a zero-only software requirement, **not** as hardwired-zero bits.
([Manual §6.2][manual-flash], [target EBR1 replacement, p. 4][addition-blocks])

| EBR1 bit | Inclusive addresses | Size |
| --- | --- | --- |
| EB0 | `0000–03FF` | 1 KiB |
| EB1 | `0400–07FF` | 1 KiB |
| EB2 | `0800–0BFF` | 1 KiB |
| EB3 | `0C00–0FFF` | 1 KiB |
| EB4 | `1000–7FFF` | 28 KiB |
| EB5 | `8000–BFFF` | 16 KiB |

Programming uses aligned 128-byte units, including `FF` padding for unwritten
bytes. Do not inherit the base manual's 12 KiB EB4 or five-block geometry.
([Target addition, pp. 3–4][addition-blocks])

## Flash progression and CPU access

This is a **CPU-controlled pulse/verify interface**. Software fills address/data
latches, selects setup and pulse modes, delays, and verifies. There is no
documented autonomous page-command/busy protocol. The system oscillator is
required for programming/erasure. The following timing obligations come from
§6.4, figures 6.3–6.4 and table 6.6 (pp. 109–113). ([Manual][manual-algorithm])

| Transition | Required delay or operation |
| --- | --- |
| `SWE=1` | 1 µs |
| Program data load | 128 consecutive byte writes latch page address/data |
| `PSU=1` → `P=1` | 50 µs setup |
| `P=1` | 30 µs for passes 1–6; their additional pulses are 10 µs; later passes use 200 µs |
| `P=0` → `PSU=0` → next operation | 5 µs, then 5 µs |
| `PV=1` | 4 µs |
| Program verify | Byte-write `FF` to a four-byte-aligned address; wait 2 µs; read word/longword |
| `PV=0` | 2 µs |
| `ESU=1` → `E=1` | 100 µs setup |
| `E=1` | 10 ms erase pulse for the selected block |
| `E=0` → `ESU=0` → next operation | 10 µs, then 10 µs |
| `EV=1` | 20 µs |
| Erase verify | Same aligned dummy write and 2 µs delay; read longword |
| `EV=0` | 4 µs |
| `SWE=0` after either algorithm | 100 µs |

The documented retry limits (1,000 program passes, 100 erase passes), comparison
and retry-data calculations belong to the software algorithm. Do not invent a
hardware retry engine. `P` requires `SWE && PSU`; `E` requires `SWE && ESU`.

During a pulse, reading the flash address being programmed/erased—including
instruction or vector fetch—starts error protection, as does a non-reset
exception or SLEEP. FLER latches; register settings remain, but the pulse aborts.
Verify remains possible; writing P/E again cannot restart programming until
reset. The manual's separate interrupt paragraph says interrupts are disabled,
then explains software/CPU failure if one occurs. **Implementation inference:**
preserve normal exception admission and apply FLER when one starts; do not
invent a global hardware interrupt mask that would defeat §6.5.3.
([Manual §§6.4.3–6.5.3, pp. 112–115][manual-protection])

Reset and transitions to subactive, subsleep, watch or standby abort programming
and initialize FLMCR1/FLMCR2/EBR1. `FROMCKSTP=0` also stops flash operation;
code must run from RAM, and flash vectors are unavailable. Subactive flash is
readable with PDWND either value; PDWND selects its power circuit state.
Returning from power-down/standby requires at least 20 µs stabilization, even
with an external clock. ([Manual §§6.5–6.7, pp. 114–116][manual-protection])

**Implementation recommendation:** retain the page latches, selected block,
mode, pulse start/elapsed duration, verify address/readiness and FLER. Feed CPU
reads, writes, fetches, exceptions and power transitions through this owner.
Treat physical pulse duration as elapsed emulated time; CPU clock changes alter
software delay execution, not the meaning of microseconds. Rebuild decode
metadata for changed bytes without replacing already-fetched CPU bytes.

A useful initial physical model programs selected bits monotonically toward
zero and erases toward one after a nominal valid pulse. Keep elapsed progress
and completed changes on interruption; aborting must never restore a previous
array or recopy the original firmware. This is a proposed nominal model, not a
claim about interrupted-pulse bit patterns. Avoid invented wear/random-corruption
machinery. Targeted measurements can refine completion thresholds locally.

Two access details need bounded inferred behavior initially: the value read
from FLSHE-disabled registers, and reads made before verify settling. Use
deterministic local behavior rather than returning a core-level unsupported
error. The documented RTS restriction between page loading and clearing P, and
between dummy writes and verify reads, is evidence of address/bus sensitivity;
do not turn it into a special prohibition on executing the RTS opcode.

## IIC2: register semantics

The map uses 8-bit, two-state accesses. The compact table below accompanies
§16.3 (pp. 314–327); qualifiers are part of the behavior, not ordinary writable
masks. ([Manual register descriptions][manual-iic], [address table][manual-registers])

| Address | Register/reset | Behavior-bearing fields |
| --- | --- | --- |
| `F078` | ICCR1 / `00` | `80 ICE`, `40 RCVD`, `20 MST`, `10 TRS`, `0F CKS`; all read/write |
| `F079` | ICCR2 / `7D` | `80 BBSY`, `40 SCP`, `20 SDAO`, `10 SDAOP`, read-only `08 SCLO`, `02 IICRST`; `05` reserved ones |
| `F07A` | ICMR / `38` | `80 MLS`, `40 WAIT`, `08 BCWP`, `07 BC`; `30` reserved ones |
| `F07B` | ICIER / `00` | `80 TIE`, `40 TEIE`, `20 RIE`, `10 NAKIE`, `08 STIE`, `04 ACKE`, read-only `02 ACKBR`, `01 ACKBT` |
| `F07C` | ICSR / `00` | `80 TDRE`, `40 TEND`, `20 RDRF`, `10 NACKF`, `08 STOP`, `04 AL/OVE`, `02 AAS`, `01 ADZ`; read-qualified write-zero clears |
| `F07D` | SAR / `00` | `FE` slave address, `01 FS` selects clock-synchronous format |
| `F07E` | ICDRT / `FF` | Transmit holding register, distinct from hidden shift register |
| `F07F` | ICDRR / `FF` | Read-only receive holding register, with read side effects |

SCP, SDAOP and BCWP always read one; writing zero enables their respective
operation. `BBSY=1,SCP=0` requests START; both zero request STOP. SDAO changes
only with SDAOP zero. BC reads remaining bits and returns zero at frame end;
zero means eight data bits, with an additional ACK clock in IIC format.
ICDRT writes and ICDRR reads have status/transfer side effects; preserve the
read qualification for software-cleared status independently of these effects.

## IIC2 progression, signals and errata

Recommended authoritative state is holding/shift data, bit and ACK phase,
master/slave direction, pending START/STOP, own open-drain drives, filtered input
latches, divider position, SCL hold reason and status-clear qualification.
Resolve these drives on **P90/SCL and P91/SDA**, including GPIO and SSU ownership.
Pin priority is SSU → IIC2 → GPIO; interrupt vector 34 is shared with SSU.
CKSTPR2 bit `20` gates IIC2; its state is retained in subactive/subsleep/watch/
standby. ([Manual pp. 43, 82, 86, 136–137][manual-pins])

Master CKS values 0–15 select these full-period divisors of φ:
`28,40,48,64,80,100,112,128,56,80,96,128,160,200,224,256`.
Retain source-clock obligations across clock changes. External/slave transfers
advance from resolved pin edges. Two cascaded system-clock samples plus a
match detector filter SCL/SDA; the bit-synchronous circuit monitors released
SCL after 7.5, 19.5, 17.5 or 41.5 φ cycles according to CKS3/CKS2. Thus a
nominal byte deadline cannot substitute for stretching and actual observed
rises. ([Manual table 16.2, §16.4.7, §16.6][manual-iic-timing])

The first frame after START is always eight address/direction bits plus ACK.
Address match/general call governs slave participation. Losing arbitration
normally clears MST/TRS and enters slave receive. Master receive starts with
an ICDRR dummy read; RCVD and subsequent reads control continuation. A full
receive register can hold SCL low at the eighth falling edge. TEND is associated
with the ninth rising edge when TDRE is set. SAR.FS also enables a real
clock-synchronous mode, with different ACK/overrun/stop behavior.
([Manual §§16.3–16.5][manual-iic-operation])

Incorporate these concrete corrections rather than implementing ideal I²C:

- **IICRST:** release SDAO/SCLO; set TDRE in transmit mode; block BBSY/SCP/SDAO
  writes during reset. It does not directly clear BBSY. START, STOP and
  arbitration detectors remain active while transfers halt. Resetting or
  disabling during an owned transaction can leave BBSY/STOP indeterminate;
  retaining their prior state unless released pins produce a detected event is
  a reasonable initial inference. Actual START/STOP repairs BBSY; SAR.FS=1
  clears it. ([A022, pp. 1–2][iic-reset])
- **STOP:** with ACKE set in master transmit, issuing STOP before the ninth
  falling edge may fail. Preserve phase when accepting the command; the safe
  sequence waits for SCLO to fall. ([A023, p. 1][iic-stop])
- **Receive race:** reading ICDRR around the eighth falling edge while RDRF is
  set can prematurely release the next frame's hold and lose data. Model the
  hold latch and same-edge order, not an unconditional register-read unblock.
  ([A017, p. 1][iic-receive])

Revision 3 already includes the WAIT/stretch interaction, multi-master rate
restriction and MST/TRS read-modify-write arbitration race in §16.7. Preserve
ordinary CPU read/write ordering so a bit instruction can overwrite a
hardware-cleared bit; do not special-case its opcode. Exact undocumented race
windows are suitable for later measurement, not grounds to reject ordinary
IIC transactions. ([Manual pp. 347–348][manual-iic-notes])

## Firmware evidence and current integration

At public `lumirth/pw` commit `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`, the
[target register header][pw-header] includes EB5 and the flash/IIC map.
[EepromRead][pw-read] and [EepromWritePage][pw-write] drive the external M95512
through SSU; [IR dispatch][pw-ir] reaches EEPROM reads/writes and ordinary
memory writes. A search of this revision's `src/` found no FLMCR/FENR/EBR1 or
IIC2 accesses, and did not identify an internal-flash programming/EEPROM code
loader. These external EEPROM routines therefore establish reached serial
behavior, not the internal-flash algorithm. Do not introduce a firmware loader
shortcut based on that conflation.

Current `mcu/mod.rs` returns zero for flash registers, rejects nonzero flash
control/array writes, and rejects the entire IIC range. Its direct flash
byte/word reads and reset-vector read also bypass any flash mode. Add small
flash and IIC owners behind those existing bus/power seams; include their causal
state in snapshots. Keep ordinary fetches cheap while checking the active flash
mode through the same execution mechanism. Exercise RAM-executed pulse/verify,
reset during a pulse, IIC stretch/arbitration, and IICRST with physical pin
activity through guest-visible behavior.

[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[addition-blocks]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=4
[manual-flash]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=135
[manual-algorithm]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=143
[manual-protection]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=148
[manual-registers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=406
[manual-iic]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=348
[manual-iic-operation]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=362
[manual-iic-timing]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=374
[manual-iic-notes]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=381
[manual-pins]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=170
[iic-reset]: https://www.renesas.com/eu/en/document/tcu/notes-use-iicrst-i2c-bus-interface-2-iic2-and-i2c-bus-interface-3-iic3#page=1
[iic-stop]: https://www.renesas.com/us/en/document/tcu/notes-about-issuance-stop-condition-master-transmit-mode-i2c-bus-interface-2iic2-and-i2c-bus#page=1
[iic-receive]: https://www.renesas.com/en/document/tcu/usage-notes-i2c-bus-interface-2-iic2-master-receive-mode#page=2
[pw-header]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/include/startup/iodefine.h#L17-L62
[pw-read]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c#L189-L300
[pw-write]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c#L507-L575
[pw-ir]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/support/ir.c#L1070-L1133
