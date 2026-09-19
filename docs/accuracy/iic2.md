# IIC2

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

IIC2 implements master/slave transfer, addressing/general call, ACK/NACK, stretching,
arbitration, receive continuation, filtered pins, status qualification and synchronous
serial mode. Resolved SCL/SDA levels feed back into the controller. Register requests
produce START/STOP through those pins. SSU and IIC2 share an interrupt vector and pin
priority follows the package diagrams.

## Limits and open questions

The model incorporates the applicable reset, STOP, synchronization and receive-hold
errata. Equal nominal clock halves and exact collision windows for some defects are
inferred where only their conditions and consequences are documented. A defect's
presence can therefore be supported more strongly than the exact emulated race window.
Multi-master contention, same-edge reads and live CKS changes remain valuable targeted
review cases. The source catalogue distinguishes these errata from general I²C advice.

## Sources

- [H8/38602R hardware manual, REJ09B0152-0300, rev. 3][manual]: §16,
  printed pp. 311–348; pin selection pp. 136–137 and appendix B.4 pp. 482–483;
  gates/power table pp. 82, 86; interrupt table p. 43. Add 34 to printed page
  numbers for PDF pages. Cached PDF/text: `out/research/h838602r-hardware.*`.
- [TN-H8*-A414A/E][addition], pp. 1–5: H8/38606F changes memory/package/flash,
  retaining the base peripheral specifications.
- [TN-MC*-A022A/E][reset], pp. 1–3: IICRST/ICE effects; names this base manual.
  [TN-MC*-A023A/E][stop], pp. 1–2: later ACKE/STOP timing defect.
- [TN-H8*-A300A/E][timing], pp. 1–2, and
  [TN-MC*-A017A/E][receive], p. 1: synchronization/WAIT and receive-hold defects,
  incorporated into the manual's §16.7.
- Renesas [REJ06B0135-0100Z, I²C EEPROM examples][eeprom], pp. 62–63, 70–72,
  and [REJ06B0106-0100Z, synchronous EEPROM example][sync], pp. 20, 22, 24:
  working receive sequences. These corroborate sequencing, not target pin maps.

## Registers and authoritative state

All registers have byte-wide, two-state accesses. The map and masks are in §16.3, pp.
314–327, and the address table on p. 373. [Manual][registers]

| Address | Register/reset | Contract |
| --- | --- | --- |
| `F078` | ICCR1 / `00` | ICE, RCVD, MST, TRS, CKS[3:0]. Setting TRS sets TDRE. Arbitration loss clears MST/TRS. |
| `F079` | ICCR2 / `7D` | BBSY follows START/STOP. Writing SCP=0 requests START if written BBSY=1, STOP if 0. SDAOP=0 permits SDAO changes. SCLO is read-only; bits 2/0 read 1. |
| `F07A` | ICMR / `38` | MLS, WAIT, BCWP, BC[2:0]. BCWP=0 permits count writes; bits 5/4 read 1. |
| `F07B` | ICIER / `00` | TIE, TEIE, RIE, NAKIE, STIE, ACKE, read-only ACKBR, ACKBT. |
| `F07C` | ICSR / `00` | TDRE, TEND, RDRF, NACKF, STOP, AL/OVE, AAS, ADZ. Per-flag read-1-before-write-0 qualification. |
| `F07D` | SAR / `00` | Bits 7:1 are the slave address; FS selects I²C (`0`) or synchronous serial (`1`). |
| `F07E` | ICDRT / `FF` | Transmit holding byte. Write clears TDRE/TEND; transfer to the shift register sets TDRE. MLS=1 reverses stored/readback bit order. |
| `F07F` | ICDRR / `FF` | Receive holding byte. Guest read clears RDRF and may release/advance reception. Writes have no effect. |

SCP, SDAOP and BCWP always read 1. BC=0 means eight data bits; values 1–7 mean that
many. I²C adds an ACK bit; its first address frame always has eight data bits. BC reads
remaining count and returns to zero after the frame. Preserve the actual bus write
strobes; instructions other than the advised MOV are not grounds for a guest fault.
[§§16.3.2–3, 16.4.1][registers]

The controller retains registers and status-read qualification; one shift register; bit/ACK
phase; current transaction selection; receive continuation/hold state; own SCL/SDA drive
intent; two input-filter histories; remaining local clock obligations. AAS is sticky
status, not the current transaction-selection latch. Use TDRE itself for
holding-register availability: manually clearing it can transmit an extra old byte
(§16.5, p. 345). Debug inspection has no read effects.

## Transfer rules

The following follows §§16.4.1–6 and flowcharts 16.17–20, pp. 328–339, 341–344. [Manual
operation and diagrams][operation]

- A filtered START sets BBSY and begins an address frame. Repeated START does
  this while the bus is already busy. Nonmatching slaves release SDA and await
  another START/STOP. General-call byte `00` selects slave receive.
- TX launches bits on SCL falling edges and samples ACK on the ninth rising
  edge. ACKBR records the bit regardless of ACKE. ACKE=1 plus a received 1
  sets NACKF and halts continuation; ACKE=0 permits continuation.
- TEND sets at the ninth rise if TDRE=1. At the ninth fall, empty TX holds SCL
  low until another byte or a START/STOP command permits progress. Writing a
  full ICDRT replaces its holding byte; it does not replace the active shift.
- Master RX starts from an ICDRR dummy read. Publish ICDRS into ICDRR and set
  RDRF at the ninth rise. If the previous RDR remains full, hold SCL low at the
  eighth falling edge, before ACK/publication; reading RDR releases it.
- RCVD is a continuation control, not an unconditional prohibition on starting
  RX. The single-byte sequence is ACKBT=1, RCVD=1, dummy-read RDR, receive one
  byte. For multiple bytes, set those bits before reading the penultimate
  byte; one final byte follows. A final read while already stopped by RCVD must
  not start another frame. Track which in-flight/next frame the read authorizes,
  including reads between the ninth rise and fall. Figure 16.18 explicitly
  specifies the single-byte case; the [EEPROM example][eeprom] confirms it.
- A matching slave write address sets AAS and publishes the address/RW byte to
  RDR at the ninth rise; software dummy-reads it. General call also sets ADZ.
  A matching read address changes TRS to TX and sets TDRE; Fig. 16.9 shows no
  corresponding receive-register publication. ACKBT controls the slave ACK.
- Slave TX stretches when starved. After data becomes available, retain
  10φ or 20φ setup time, selected by CKS3, before releasing SCL. Slave RX
  uses the eighth-fall unread-RDR hold. Clearing TRS and dummy-reading RDR
  releases a completed slave transmitter; TEND alone does not do so.
- STOP clears BBSY. STOP status qualifies after a completed master frame or an
  addressed/general-call slave transaction. STOP does not automatically clear
  MST/TRS: the flowcharts perform that write in software.
- Arbitration loss compares transmitted data with resolved SDA on SCL rises,
  excluding ACK reception, and detects another START while master SDA intent
  is high. Set AL, clear MST/TRS and release master ownership. Retaining the
  partially shifted address as slave reception continues is the chosen inference.

FS=1 reuses the shift/holding engine without address or ACK phases. Sample RX on rises,
change TX on falls. Master RX starts when master receive is selected, without an I²C
dummy read. Publish on the eighth rise. If RDRF is still set, preserve the previous RDR,
set OVE and clear MST. RCVD/read sequencing ends continuous reception with SCL high. The
[synchronous example][sync], pp. 20, 22, 24, warns that setting RCVD immediately after
entering master receive can suppress the first clock; it waits for reception to start
first.

## Clocks, pins, reset and IRQ

CKS selects a private generator driven by system φ, not a prescaler-S tap:

```text
CKS 0..7:  28, 40, 48, 64, 80, 100, 112, 128 φ cycles per bit
CKS 8..F:  56, 80, 96,128,160,200,224,256 φ cycles per bit
```

Two cascaded φ-sampled latches filter each pad: update the filtered value only when the
latches agree, otherwise retain it. START/STOP and receive edges use these filtered
inputs. Master synchronization monitors released SCL after `7.5, 19.5, 17.5, 41.5φ` for
CKS3/CKS2=`00,01,10,11`. Preserve half-cycle phase. External low prevents a released
clock from advancing as high.
[Table 16.2; §§16.4.7, 16.6, pp. 316, 340, 346][clock]

P90/SCL and P91/SDA are open-drain, with SSU > IIC2 > GPIO priority. SDAO/SCLO report
output intent; SDAI/SCLI are distinct pad inputs in the pin diagrams. Feed resolved pads
back even when another function masks the IIC output; do not equate release with a high
pad. CKSTPR2 bit5 enables IIC2; clearing it halts the module. State is retained in
watch, subactive, subsleep and standby. Preserve phase across clock absence; MCU reset
restores the table.
[Manual pp. 82, 86, 136–137; appendix B.4, pp. 482–483][pins]

IRQ34 is shared with SSU. Use:

```text
TDRE&TIE | TEND&TEIE | RDRF&RIE
| (FS=0 && STOP&STIE)
| (NAKIE && (AL_or_OVE || (FS=0 && NACKF)))
```

Table 16.3 and the NAKIE description put AL/OVE under NAKIE. RIE prose contradicts this;
follow the table/NAKIE definition. Existing flags plus enables remain the request
source, independently of whether the transfer clock runs.
[§§16.3.4, 16.5, pp. 321–322, 345][irq]

IICRST releases SDAO/SCLO and sets TDRE in TX mode; it does not reinitialize the
registers. While held, BBSY/SCP/SDAO writes are blocked and shifting stops, but
START/STOP/arbitration detection remains active. BBSY is not automatically cleared: a
resulting physical STOP may clear it. Writing FS=1 clears BBSY. During active reset/ICE
disable, BBSY/STOP are documented as indeterminate; choose to retain them first, then
apply actual detected edges. [A022, pp. 1–2][reset]

## Chosen boundaries and silicon defects

These choices complete the model where the sources constrain outcomes without specifying
every internal phase. They are local implementation inferences:

- Use equal nominal high/low halves of the documented full period. At the
  specified monitor offset, pause the remaining high obligation if SCL is low;
  resume when its filtered input is high. The sources do not provide a complete
  generated duty-cycle table. A live CKS write preserves the current obligation
  and takes effect at the next reload.
- Apply live direction/order changes at the next relevant edge, retaining
  shifted bits. Accept START/STOP strobes into their physical sequence at a low
  boundary; do not manufacture a completed condition while a pad is held low.
  Preserve normal CPU read/write ordering for the documented MST/TRS
  read-modify-write arbitration race.
- Do not queue every early STOP safely: with ACKE=1 in master TX, a request
  before the ninth fall can fail ([A023, p. 1][stop]). START/STOP also interact
  with synchronization during stretch; WAIT=1 extends the pre-ACK low interval
  by two transfer periods and can shorten ACK high under the documented
  stretch condition ([§16.7.1–3][notes], [A300][timing]). A deterministic early-request loss
  at the ninth-high collision is the selected collision rule, not measured timing.
- An RDR read around the eighth fall can release the following frame's hold
  without another read, losing data ([A017, p. 1][receive]). A single stale
  release credit models the effect. Choose the collision on the same φ sampling
  edge and release on the next φ sample; this exact window is an inference.

Keep chosen-boundary regressions separate from expectations established by the manual.
The legal byte sequences work through this same model; no unsupported guest operation
branch or alternate atomic-byte execution path is needed.

## Original conformance vectors

Unless specified otherwise, start from reset in active mode, enable CKSTPR2 bit5,
disable SSU with bit4=0, and attach an open-drain signal fixture to physical P90/P91.
Give each input level enough φ samples to pass the filter. Drive data while SCL is low.
The expected results below exclude the chosen race windows.

| Case | Guest actions / input | Expected observations and basis |
| --- | --- | --- |
| `iic-reset-map` | Read `F078..F07F` after MCU reset. | `00 7D 38 00 00 00 FF FF`; §16.3. |
| `iic-lsb-holding` | Set MLS=1; write ICDRT=`01`, read ICDRT. | `80`; §16.3.7. No transfer is needed to observe reversal. |
| `iic-address-and-count` | Issue START; set BC=3; send `A0`; then set BC=3 and send `A0` as data. Fixture ACKs. | Address clocks eight bits `10100000` plus ACK; data clocks `101` plus ACK; BC returns to zero. §16.4.1 and BC table. |
| `iic-single-read` | After acknowledged read address: clear TEND/TRS/TDRE; set ACKBT=1, RCVD=1; dummy-read RDR. Fixture sends `A5`. | Eight data rises plus ninth-bit NACK; RDRF set and RDR=`A5`; no further frame. Reading final RDR does not rearm. Fig. 16.18. |
| `iic-receive-hold` | Receive `3C`, leave RDR unread; fixture supplies `A5` next. | RDR stays `3C`; SCL holds at the next eighth fall before ACK. Read `3C`; ninth rise publishes `A5`. §§16.4.3, 16.4.5. |
| `iic-slave-selection` | SAR=`54`, slave RX, ACKBT=0. Separate transactions send addresses `54`, `55`, `00`, `56`. | `54`: AAS/RDRF, RDR=`54`; `55`: AAS/TRS/TDRE; `00`: AAS/ADZ/RDRF, RDR=`00`; `56`: no selection/ACK. Clear sticky flags between transactions. §§16.3.5–6, 16.4.4–5. |
| `iic-nack-control` | Master TX queues another byte before ACK; fixture returns 1. Repeat with ACKE=0/1. | Both record ACKBR=1; only ACKE=1 sets NACKF and stops continuation. §16.3.4–5. |
| `iic-arbitration` | Master transmits `A0`; during the first data-low phase another master holds SDA low, continuing through the following rise. | Intended first bit is 1 but resolved bit is 0: AL sets and MST/TRS clear. This is a data bit, not the ACK phase. §16.3.5. |
| `iic-stop-mode` | Complete master TX; wait for ninth fall, request STOP. | BBSY clears and STOP sets; MST/TRS remain set until software clears them. Fig. 16.17; A023 safe sequence. |
| `iic-sync-overrun` | FS=1, master RX, RIE=0, NAKIE=1; receive `3C` then `C3` without reading RDR. | RDR=`3C`, RDRF/OVE set, MST cleared, IRQ34 requested. §16.4.6 and table 16.3. |
| `iic-filter` | With filtered SCL/SDA high, supply sampled SDA history `1,0,1`; then `1,0,0`. | First history produces no START; second produces START after the agreeing low samples. §16.4.7. |
| `iic-reset-detector` | During IICRST, externally hold SDA low, bring SCL high, then release SDA. | Own drives are released; the filtered STOP clears BBSY despite IICRST. A022. |
| `iic-clock-period` | Observe repeated un-stretched data clocks at CKS0, then CKS5. | Full periods are 28φ and 100φ, ratio `25/7`; independent calculation from table 16.2. Do not assert inferred duty split. |

IIC drives resolve on the same P90/P91 pads as GPIO and SSU. P90 also selects the sensor
and P91 clocks the shared serial bus, so transitions reach those external chips too.
PFCR.SSUS relocates SSU functions, not IIC pads. Save states retain filter, shift, hold
and clock progress; appointments are derived from it.

## Implementation and checks

The [IIC2 implementation](../../crates/hs-core/src/mcu/iic.rs) owns its controller;
[GPIO](bus-and-gpio.md) owns pad selection. Hachiware's
[IIC cases](https://github.com/lumirth/hachiware/blob/main/cases/iic.py) check reset,
holding order, flag clearing, NACK/STOP and slave/general-call reception.
Local [IIC tests](../../crates/hs-core/tests/iic.rs) cover further controller behavior;
[machine tests](../../crates/hs-core/tests/kernel.rs) check input histories and restoration.
The example vectors above describe useful expected behavior; their presence in this
page does not mean each has an independent guest case. Exact erratum collision windows
remain the stated model choices.

[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group
[registers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=348
[operation]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=362
[clock]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=374
[notes]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=381
[pins]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=516
[irq]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=379
[reset]: https://www.renesas.com/en/document/tcu/notes-use-iicrst-i2c-bus-interface-2-iic2-and-i2c-bus-interface-3-iic3
[stop]: https://www.renesas.com/us/en/document/tcu/notes-about-issuance-stop-condition-master-transmit-mode-i2c-bus-interface-2iic2-and-i2c-bus
[timing]: https://www.renesas.com/en/document/tcu/usage-notes-i2c-bus-interface-2-iic2
[receive]: https://www.renesas.com/en/document/tcu/usage-notes-i2c-bus-interface-2-iic2-master-receive-mode
[eeprom]: https://www.renesas.com/en/document/apn/application-examples-reading-fromwriting-serial-eeprom
[sync]: https://www.renesas.com/en/document/apn/access-serial-eeprom-spi-eeprom-clock-synchronous-mode-i2c-interface
