# BMA150 I²C

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The sensor's I²C interface operates on the existing P91/P92 nets while CSB is high.
Addressing, ACK/NACK, incrementing reads and paired address/data writes share the same
registers, conversion state and read side effects as SPI. The MCU's IIC2 controller
uses different pins; sensor I²C is available through GPIO on this board.

## Limits and open questions

The byte protocol and fixed address have direct datasheet support. Sleep ACKs, read
acknowledgement edges and pointer retention across interface changes include selected
inferences explained below. Pad slew and pull-up strength are not modeled, so accepting
a fast digital transaction does not establish that the board can sustain its bit rate.
Sampling, calibration and interrupt limits belong to the [sensor model](bma150.md).

## Reachability and electrical ownership

| Board net | H8 pin | BMA150 pin/function | I²C ownership |
| --- | --- | --- | --- |
| Sensor select | P90 | 5, CSB | Keep high; low selects SPI |
| Shared serial clock | P91 | 6, SCK | MCU drives; sensor input only |
| Shared MOSI | P92 | 8, SDI/SDA | Master and sensor independently pull low or release |
| Shared MISO | P93 | 7, SDO | Sensor releases in I²C; not an address strap |

`pw` selects four-wire SPI, drives CSB with PDR9 bit 0 and enables only the P93 pull-up.
The H8 pin map places default SSCK/SSO/SSI on P91/P92/P93, respectively. Together these
establish the board mapping used by the current resolver. The public board investigation
identifies the BMA150 and shared SPI nets but supplies no measured SDA pull-up or
complete sensor netlist. ([`pw` setup][pw-setup], [`pw` sensor routines][pw-sensor],
[H8 §8.4, p.132][h8-pins], [board investigation][board], [device identification][parts])

Bosch §4.2.1, p.36 fixes the seven-bit address at 38, yielding wire bytes 70/71. This is
a metal option, not SDO selection. Table 12, p.43 recommends grounding SDO in an
I²C-only circuit; it describes SDO as an output. Its existing connection to MCU P93 does
not prevent I²C. SPI4, register 15 bit7, chooses only three-/four-wire SPI; it does not
disable I²C. ([Protocol][p36], [pin table][p43],
[SPI4][p11])

Use CSB as a live interface selector. CSB low aborts any I²C transaction and starts SPI;
CSB high releases SPI output and permits a fresh I²C START. This is the compact circuit
inference supported by §4.1.1's explicit ban on SDI changes while SCK and CSB are both
high: those changes are I²C START/STOP conditions. No source here specifies a permanent
SPI-mode latch after the first CS assertion. ([SPI selection][p25], [I²C
selection][p32])

The H8 hardware IIC pins are P90=SCL, P91=SDA, even when SSUS relocates SSU signals.
They do not match the sensor's P91/P92 pair. Preserve that fact; do not connect the
sensor directly to `mcu/iic.rs`. A GPIO master must disable SSU/IIC ownership and leave
the other serial devices deselected. The BMA150 never drives SCK, so it does not
clock-stretch; arbitration is a master's observation of the resolved SDA, not another
sensor mechanism. ([H8 pin map][h8-pins],
[Bosch pin types][p43])

A physically available pull-up is the H8's own PUCR92. Keep PDR92=0 and toggle PCR92:
output drives low; input releases, enables the configured pull-up and lets PDR92 read
the pad. Set PUCR92=1 before releasing it. Merely setting PODR92 and writing a high
output leaves PCR92=1, which both disables that internal pull-up and makes PDR92 read
its latch. H8 §§8.4.1–4, pp.133–134 specify these distinctions. The same direction/pull
method can operate SCL; a push-pull master clock also works with this input-only slave.
Actual pull strength/capacitance limits the physical bit rate; no external P92 pull-up
or nanosecond rise-time measurement has been established. ([H8 GPIO rules][h8-gpio])

## Protocol contract

The following comes from Bosch §§4.2–4.2.1, pp.32–37, figures 11–16; byte and register
values below are hexadecimal. ([Bus edges][p34], [writes][p36], [reads][p37])

- A falling SDA while SCL remains high starts or restarts an address frame;
  a rising SDA while SCL remains high stops it. Sample bytes MSB first on
  SCL rises. Data and ACK drive changes belong to SCL-low phases.
- After START, accept only 70 or 71. ACK a match by pulling SDA low for the
  ninth clock. A mismatch, including general call 00, releases SDA and ignores
  subsequent bytes until START/STOP. SDO level does not alter address matching.
- After 70, receive a control byte, using bits6:0 as register address and
  ignoring bit7. Receive one data byte and apply it to that register. Then
  expect another control byte. Multi-write is repeated address/data pairs,
  not an automatically incrementing stream of data.
- A control byte followed by STOP sets the retained read pointer. Figure 16
  explicitly uses STOP followed by START/71. Accept repeated START/71 too:
  table 11 explicitly specifies repeated-START timing, and retaining the
  pointer without an intervening STOP is the coherent circuit interpretation.
- After 71 and its ACK, transmit at the retained pointer. A master ACK requests
  the next incremented address; a master NACK ends output until START/STOP.
  The ninth clock is not data and must not cause another register read.
- Protected address and data writes are still ACKed but ignored when locked.
  A protected read releases SDA for all eight bits, yielding FF with a pull-up;
  its ACK/NACK is still the master's. This is explicit in §3.3.3, p.20 and must
  remain separate from whether `peek`/`write_register` permit an access. ([Protection][p20])

Choose a seven-bit pointer that advances after each fully clocked read byte, including
the final NACKed byte; wrap 7F to 00. Retain it through STOP and repeated START. These
completion/wrap details are circuit inferences from the seven-bit address and automatic
increment, not separately specified rules. Cold/soft reset initializes it to zero.
Retain it on CSB mode changes, while discarding partial I²C frames; whether SPI accesses
overwrite an I²C pointer is not documented. Firmware can remove this ambiguity by
sending the pointer before reading, as the manufacturer's procedure does.

## Reuse and causal state

Keep one register bank and reuse `prepare_read`, `acknowledge_read` and
`write_register`. The additional transport needs only a transaction phase
(idle/address/control/data/read/ACK/ignore), 7-bit pointer, bit count and shift byte,
ACK/continuation state, previous resolved SCL/SDA, and SDA-low intent. Reuse the
existing prepared-byte/shadow state where convenient. This state must survive an exact
save/restore; no I²C-specific register image or oscillator is required.

Call the sensor pin observer whenever CSB, SCL or SDA changes. Resolve device SDA as
low/released on P92 through the existing `serial_data` path, then settle the resulting
pad value at the same emulated instant. Remember the final resolved levels; a slave's
own ACK release must not be mistaken for a host-generated START/STOP. A sensor one bit
is release, never `Drive::High`.

Use these edge boundaries as the nominal digital realization:

1. Latch a received byte at its eighth rising edge. An accepted data byte
   invokes the existing write once. Assert ACK after the eighth falling edge,
   hold through the ninth rise, and release after the ninth fall.
2. Launch a read byte after the preceding ACK's falling edge. Prepare its
   byte/shadow once; call `acknowledge_read` on its first data sampling rise,
   matching the existing SPI inference. The master's later ACK is a different
   operation. A one-byte NACKed read still clears freshness; an unclocked
   prepared next byte does not. Increment only after all eight data bits.
3. Sample master ACK on rise nine. If low, launch the next byte after fall
   nine; if high, release and wait. START/STOP discard partial incoming bytes
   without writes, while already clocked bytes retain their effects.

The datasheet defines the byte-level shadow/freshness effects but not their exact
internal acknowledgement edge. Reusing the current first-data-edge choice avoids a
second sensor read policy. ([§§3.1.6/3.5.2–3][p23])

Keep the address/write decoder available in sleep so the documented wake or soft-reset
command can be received. Choose normal bus ACKs for those frames; reuse current register
policy for forbidden sleeping traffic (ignored writes, undriven reads). Do not gate
every I²C ACK with `!sleeping()`. Retain the final ACK of an accepted sleep/reset write
even if its action gates subsequent traffic; otherwise a successful wake/reset command
can appear to fail. This ACK choice outside ordinary active operation is inferred, not
measured. Unpowered/cold-not-ready devices release SDA and discard partial frames; new
traffic during the existing 10 µs soft-reset quiet interval is unacknowledged. Require a
new START after readiness returns. ([Sleep/reset][p21], [modes][p46])

## Timing

Rev. 1.7 §4.2, table 11, p.34 specifies up to 3.4 MHz, SCL low ≥160 ns/high ≥60 ns, SDA
setup ≥10 ns, SDA hold 0–70 ns, repeated-START setup/start hold/STOP setup ≥160 ns, and
bus-free ≥100 ns. There is no extra SPI-style turnaround clock, serial byte timeout, or
I²C-specific register-write delay. Existing cold/wake/reset/image/EEPROM readiness
continues through the shared owner. ([Corrected timing][rev17-timing])

The 29 June 2010 revision corrects the older 10 ns minimum; its history explicitly
identifies the `Thddat` entry as a typo. This changes the cited input timing
requirement, not the existing edge-level implementation. ([Revision history,
p.58][rev17-history])

For this digital interface, launch output on the falling-edge consequence and sample on
the next rise. A valid 160 ns low interval exceeds the specified 70 ns maximum
output-data hold. This abstracts pad propagation and pull-up slew rather than inventing
a calibrated pad delay. Exact sub-edge pad timing and the maximum bit rate with the
board's pull-up are physical parameters that could refine the digital model.

## Independent guest expectations

Use GPIO, keep CSB high and LCD/EEPROM deselected, enable a released-SDA pull-up, and
observe PDR92 with PCR92=0. `S/P/Sr` mean START/STOP/repeated START; `A/N` mean
ACK/NACK; the master supplies A/N after read data. Wait for readiness.

| Stimulus | Expected observation |
| --- | --- |
| `S 70 A 00 A P S 71 A [read] N P` | Chip-ID byte 02 for the modeled unit; manufacturer-defined bits2:0 are 2. |
| Replace pointer 00 with 80, and `P S` with `Sr` | Same chip-ID result; control bit7 is dummy. |
| `S 72 [ACK slot] P` | NACK. Repeat with P93 driven low/high: 70 still ACKs and 72 still NACKs. |
| `S 70 A 0C A 20 A 0D A 02 A P`; then pointer 0C and read two bytes | Read `20 A 02 N`; demonstrates address/data-pair writes and incrementing reads. |
| EE_W=0: `S 70 A 16 A A5 A P`; read pointer16 | Every write frame ACKs; read FF because access is blocked. |
| Unlock via `0A,10`, write `16,A5`, read pointer16 | All frames ACK; read A5. |

Additional distinguishing cases: NACK after the first byte must stop output; nine
further clocks without START must see a released bus. STOP after a partial write data
byte must leave the register unchanged; a completed data byte remains written even
without STOP. Hold CSB low and send only the 70 address frame: the SDA ACK slot is
released; raising CSB and issuing a fresh START makes 70 ACK again. A prepared but
unclocked acceleration byte leaves freshness/shadows untouched; after its first data
rise, freshness clears even if the master ultimately NACKs, assuming no intervening
conversion. A split run or save/restore in either ACK half, between pointer and repeated
START, or mid-read must reproduce the same bytes and pad transitions.

## Implementation and checks

The [I²C parser](../../crates/hs-core/src/devices/bma150/i2c.rs) shares its
[sensor owner](bma150.md) with SPI. Hachiware's
[sensor I²C cases](https://github.com/lumirth/hachiware/blob/main/cases/sensor_i2c.py)
operate through guest GPIO and check the protocol on the actual modeled nets.
Local [sensor I²C tests](../../crates/hs-core/tests/sensor_i2c.rs) add transport and
state continuity checks. These exercise digital transactions and the selected read
boundaries; they do not characterize pull-up rise time.

[p11]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=11
[p20]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=20
[p21]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=21
[p23]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=23
[p25]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=25
[p32]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=32
[rev17-timing]: https://datasheet.datasheetarchive.com/originals/library/Datasheets-EDS4/DSAEDA00066689.pdf#page=34
[rev17-history]: https://datasheet.datasheetarchive.com/originals/library/Datasheets-EDS4/DSAEDA00066689.pdf#page=58
[p34]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=34
[p36]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=36
[p37]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=37
[p43]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=43
[p46]: https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=46
[h8-pins]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=166
[h8-gpio]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=167
[pw-setup]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_ssu_init.c#L4-L19
[pw-sensor]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_accel_bma150.c#L18-L94
[board]: https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/doc/Board.md
[parts]: https://dmitry.gr/?proj=28.+pokewalker&r=05.Projects
