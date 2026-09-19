# Hardware sources and provenance

This catalogue collects the sources used to understand the Pokéwalker and leads that
can refine its model. [ACCURACY](ACCURACY.md) describes the implementation's supported
behavior and limits. The linked hardware notes explain source conflicts and the
reasoning behind individual choices.

The main catalogue includes references used in the hardware and design notes.
Additional research leads identify material located for further examination; listing
one does not mean its claims have been applied to HachiStep. Exact sections and code
locations remain cited beside the mechanism they support.

Use the target and component revision when interpreting a source. A vendor's example
program can corroborate sequencing and provide a guest workload. A related chip's
diagram can support a circuit inference. Neither automatically establishes every
H8/38606 register or a Pokéwalker board connection. When sources disagree, preserve
the disagreement and the reason for the selected interpretation.

## Contents

- [MCU manuals and amendments](#mcu-manuals-and-amendments)
- [MCU application notes](#mcu-application-notes)
- [Instruction encodings and independent implementations](#instruction-encodings-and-independent-implementations)
- [Accelerometer](#accelerometer)
- [Display controller](#display-controller)
- [EEPROM](#eeprom)
- [Board and firmware](#board-and-firmware)
- [Infrared components](#infrared-components)
- [Emulator design and validation](#emulator-design-and-validation)
- [Additional research leads](#additional-research-leads)
- [Supplied images](#supplied-images)
- [Supplied document identities](#supplied-document-identities)
- [Detailed hardware notes](#detailed-hardware-notes)

## MCU manuals and amendments

### Target and instruction manuals

| Source | What it establishes or helps resolve |
| --- | --- |
| [H8/38602R group hardware manual](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual), REJ09B0152-0300, rev. 3.00, May 2007 | Baseline CPU restrictions, peripheral registers, pin functions, access timing, clocks, reset domains and electrical tables. Read with the H8/38606 addition. Printed pages are PDF page ordinals minus 34 in the reviewed English copy. |
| [Addition of H8/38606 group](https://www.renesas.com/en/document/tcu/addition-h838606-group), TN-H8*-A414A/E, April 2009 | Target memory map, package and flash differences. Replaces base-part geometry with 48 KiB flash, 2 KiB RAM and six erase blocks. |
| [H8/300H software manual](https://www.renesas.com/en/document/mah/h8300h-series-software-manual), REJ09B0213-0300, rev. 3 | Instruction semantics, encodings, flags, access sequences and cycle counts. Sections 2.4 through 2.8 are especially useful when individual instruction prose conflicts with consolidated tables. Printed pages are PDF ordinals minus 16 in the reviewed copy. |
| Hitachi H8/300H programming manual, ADE-602-053A, supplied copy identified below | Earlier instruction descriptions and MOV alias notes. Compare with the Renesas manual and the actual target rather than treating different editions as interchangeable. |
| [Japanese H8/38602R hardware manual](https://www.renesas.com/ja/document/mah/h838602r-group-hardware-manual) | Original-language check for ambiguous English descriptions, including register access. Find sections by name; English PDF offsets do not apply. |
| [H8/3318 manual](https://www.renesas.com/en/document/mah/h83318), normal-mode exception stack description | Supporting evidence for the inherited duplicated CCR stack-byte format. This older CPU does not define the target's peripheral behavior. |
| [H8/3048B hardware manual](https://www.renesas.com/en/document/mah/h83048b-group-hardware-manual), register-bus diagrams | Supporting evidence for byte lanes and peripheral accesses. Used narrowly in the register-access interpretation, not as a substitute target map. |

The main manual's operation, timing-diagram, usage-note and electrical sections answer
different questions. A feature list alone omits access conflicts and transition rules.
Useful entry points are §3.8 for interrupt conflicts, §§4–5 for clocks/modes, §6 for
flash, §8 for GPIO, §§9–18 for peripherals, §19 for reset, §20 for register/reset
tables, §21 for electrical limits, and the appendices for instructions and pin logic.
The hardware notes give exact pages for the claims derived from them.

### Corrections and silicon defects

| Source | Applicability and use |
| --- | --- |
| [H8/38602 specification changes](https://www.renesas.com/en/document/tcu/h838602-group-specification-changes), TN-H8*-A287A/E, November 2004 | Clock stabilization, register/electrical corrections, comparator response and flash qualification. Many changes are incorporated into rev. 3; keep the update to understand their origin and wording. |
| [SCI3 specification change](https://www.renesas.com/us/en/document/tcu/about-sci3-specification-change-0), TN-H8*-A333B/E, July 2006 | Removes multiprocessor operation and defines five-bit formats. Revision B supersedes A; the target manual incorporates the change. |
| [Watchdog timer usage note](https://www.renesas.com/en/document/tcu/h838086r-group-h838076r-group-h838602r-group-watchdog-timer-usage-note-0), TN-H8*-A309B/E, revision 2, October 2005 | Instruction-address-dependent MOV.B absolute-8 write defect and affected register fields. Use B rather than the older A description. |
| [IICRST usage](https://www.renesas.com/en/document/tcu/notes-use-iicrst-i2c-bus-interface-2-iic2-and-i2c-bus-interface-3-iic3), TN-MC*-A022A/E | IICRST/ICE reset effects, retained registers, output release and continuing bus detection. The note names the base manual. |
| [STOP issuance in master transmit](https://www.renesas.com/us/en/document/tcu/notes-about-issuance-stop-condition-master-transmit-mode-i2c-bus-interface-2iic2-and-i2c-bus), TN-MC*-A023A/E | ACKE/STOP collision. Establishes a failure condition; the exact collision window in HachiStep remains an inference. |
| [IIC2 usage notes](https://www.renesas.com/en/document/tcu/usage-notes-i2c-bus-interface-2-iic2), TN-H8*-A300A/E | Clock synchronization and WAIT behavior, also incorporated into §16.7. |
| [IIC2 master receive usage](https://www.renesas.com/en/document/tcu/usage-notes-i2c-bus-interface-2-iic2-master-receive-mode), TN-MC*-A017A/E | Receive-hold release defect near an RDR access. Relevant to data loss and clock stretching. |
| [SSU usage notes for H8SX/1520 and H8SX/1582](https://www.renesas.com/en/document/tcu/usage-notes-ssu-h8sx1520-group-and-h8sx1582) | Related-controller comparison only. The identified parts differ from H8/38606; the note does not establish that this target has the same defect. |

## MCU application notes

These sources have informed the detailed notes. Sample setup and service routines
remain software executed by a guest. Their usefulness includes register ordering,
independent examples and explanations of the hardware behind a recommended sequence.

| Source | Relevant questions |
| --- | --- |
| [Technical Q&A, H8/300H series](https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note) | QA300H-015A on interrupt enable changes, -021A on reset deferral, -033A on decimal adjustment and -037A on STC.W's unspecified byte. Check product scope when applying older examples. |
| [RTC operation](https://www.renesas.com/en/document/apn/h838602r-group-rtc-operation), REJ06B0515-0100, March 2005 | Same-target busy/data timing, INT selection and watch/subactive interrupt sequence. Does not fix the initial BSY phase after RUN. |
| [A/D conversion using subclock](https://www.renesas.com/en/document/apn/h838602r-group-application-note-ad-conversion-using-subclock), REJ06B0514-0100, March 2005 | ADC source, mode transition and RTC interrupt configuration. Its RTCCSR table also supplies evidence for the broader calendar-source decode. |
| [Comparator with internal reference](https://www.renesas.com/en/document/apn/h838602r-group-application-note-voltage-comparison-comparator-internal-voltage-reference), REJ06B0511-0100, March 2005 | Ladder selection and hysteresis; resolves the manual's conflicting threshold prose. |
| [Comparator with external reference](https://www.renesas.com/en/document/apn/h838602r-group-application-note-voltage-comparison-comparator-external-voltage-reference) | VCref pin selection, settling and read/flag sequence. |
| [Asynchronous serial transmission](https://www.renesas.com/en/document/apn/serial-data-transmission-asynchronous-mode-rej06b0247-0100z), REJ06B0247-0100Z, December 2003 | Buffer and TDRE/TEND relationships. The H8/300L example does not override target SCI3 register rules. |
| [Synchronous serial master reception](https://www.renesas.com/en/document/apn/clock-synchronization-serial-data-master-reception), REJ06B0371-0100Z | Receive-only master clocking and continuous reception, especially pp. 6 and 12–14. |
| [Infrared communication using IrDA](https://www.renesas.com/in/en/document/apn/h8300h-slp-series-application-note-infrared-communication-using-irda), REJ06B0431-0100 | UART/IrDA datapath and working communication sequence, pp. 2–9. Related target details require comparison with H8/38606. |
| [Reading/writing serial EEPROM](https://www.renesas.com/en/document/apn/application-examples-reading-fromwriting-serial-eeprom), REJ06B0135-0100Z | I²C final-byte and receive sequencing, especially pp. 62–63 and 70–72. |
| [SPI EEPROM through synchronous I²C-interface mode](https://www.renesas.com/en/document/apn/access-serial-eeprom-spi-eeprom-clock-synchronous-mode-i2c-interface), REJ06B0106-0100Z | Synchronous receive start/continuation and RCVD timing, especially pp. 20, 22 and 24. |

## Instruction encodings and independent implementations

- [GNU binutils opcode definitions](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/include/opcode/h8300.h),
  [MOV.L assembler input](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/gas/testsuite/gas/h8300/movlh.s)
  and [expected encodings](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/gas/testsuite/gas/h8300/h8300.exp)
  at commit `8ea833b706790cbf50ef3b46028a9ea0d8ecd462`. These provide independently
  assembled encodings and help resolve contradictory selector tables.
- [MAME H8 instruction definitions](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.lst)
  and [ALU implementation](https://github.com/mamedev/mame/blob/57018adb9d8cd92949081fade9ad0ba3038dbf37/src/devices/cpu/h8/h8.cpp)
  at commit `57018adb9d8cd92949081fade9ad0ba3038dbf37`. Useful comparisons for branch
  prefetch, MOV ordering, division and decimal arithmetic. The HachiStep notes record
  disagreements with MAME, including alias and flag behavior. Agreement between
  emulators alone does not establish a silicon result.

See [encoding/arithmetic](research/h8-encoding-and-arithmetic.md) and
[execution](research/h8-execution.md) for the selected interpretation of each conflict.

## Accelerometer

| Source | Relevance and limits |
| --- | --- |
| [Bosch BMA150 datasheet](https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf), BST-BMA150-DS000-06, rev. 1.6, October 2008 | Register map, conversion, filtering, interrupt logic, power modes, serial interfaces, timing and electrical characteristics. Figures 3/29 and §§3–4 are central to the model. |
| [Bosch BMA150 rev. 1.7](https://datasheet.datasheetarchive.com/originals/library/Datasheets-EDS4/DSAEDA00066689.pdf), BST-BMA150-DS000-07, June 2010, archived manufacturer document | Useful revision comparison, including I²C timing and revision history. Still omits the numerical gain/temperature-trim transfer. A later document does not imply a new Pokéwalker hardware revision. |
| [Bosch-authored BMA150 driver/calibration code](https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c) | Offset encoding, correction scale, calibration range and EEPROM timing. The mirror preserves vendor code used by the sensor note. |
| [SMB380 API implementation](https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.c) and [header](https://github.com/drakaz/gaosp_kernel/blob/c2703148748cecfe450955ad189c635846c143f7/drivers/i2c/chips/smb380.h) | Protected fields, mode setter and status definitions. Relevant because the Bosch/Linux driver identifies compatibility with BMA150 apart from packaging. |
| [Linux BMA150 driver](https://kernel.googlesource.com/pub/scm/linux/kernel/git/tj/sched_ext/+/d023aa69c3b5f22a442fb67a37f17b04602eb43f/drivers/input/misc/bma150.c) | Compatibility statement and working driver configuration. A driver demonstrates use of a function rather than all possible register effects. |
| [Atmel/Microchip ASF BMA150 driver](https://github.com/avrxml/asf/blob/68cddb46ae5ebc24ef8287a8d4c61a6efa5e2848/common/services/sensors/drivers/bosch/bma150.c) and [register definitions](https://github.com/avrxml/asf/blob/68cddb46ae5ebc24ef8287a8d4c61a6efa5e2848/common/services/sensors/drivers/bosch/bma150.h) | Wake configuration, acquisition formulas, field definitions and another driver interpretation. Compare its wake-mode setter with Bosch's rather than assuming they are identical. |
| [Bosch inline-calibration note](https://www.bosch-sensortec.com/media/boschsensortec/downloads/application_notes_1/bst-mas-an030.pdf), 2019 | Examined as a possible source for calibration. It applies to different parts/register layouts and does not supply the missing BMA150 transfer function. |

[Sensor behavior](research/bma150-behavior.md) reconciles status-bit, wake-duration and
mode-setting conflicts. [Sensor I²C](research/bma150-i2c.md) explains board reachability,
the fixed address, interface selection and timing revisions. The firmware driver is
listed under [board and firmware](#board-and-firmware). The unrecovered older
calibration note is listed under [research leads](#unrecovered-or-unidentified-material).

## Display controller

[Novatek NT7508 V1.0](https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf),
July 2008, is the controller reference. Useful sections include:

| Pages | Questions answered |
| --- | --- |
| 6, 8, 11–16 | Datapath, serial interface, select/byte handling, RAM addressing and latches. |
| 18–23 | PWM/FRC, scan and polarity, COM/SEG voltage relationships. |
| Supply-circuit description and 41–44 | Converter, regulator, follower, bias, booster and display controls. These matter to the known electrical-control gap. |
| 29–32, 33–51 | Reset lists, command inventory, geometry, oscillator control, contrast trim and OTP conditions. |
| 53, 56–57 | Reset and electrical/oscillator timing; distinguish guaranteed bounds from typical values. |

The [LCD note](research/lcd-and-eeprom.md) resolves conflicts in wrapping and reset
descriptions and relates the firmware's initialization to scan timing. The controller
datasheet establishes digital and electrical functions. It does not supply a calibrated
response curve for the actual Pokéwalker glass.

The supplied archive also contains an SSD1854 datasheet. The presence of that file
does not identify the installed controller. Its identity is retained below for
provenance; NT7508, board evidence and the matching driver determine this model.

## EEPROM

- [ST M95512-R datasheet](https://www.st.com/resource/en/datasheet/m95512-r.pdf),
  DS4192 rev. 24 in the existing research. The board investigation identifies M95512RP.
  Relevant material includes READ/WRITE, WREN/WIP, WRSR/protection, page wrap and
  power transitions. The [EEPROM note](research/lcd-and-eeprom.md) cites the sections.
- [ST M95512 family datasheet](https://www.st.com/resource/en/datasheet/m95512-w.pdf)
  and the [archived family copy used for drive limits](https://www.mouser.com/datasheet/2/389/m95512-w-955061.pdf).
  The family includes variants with different voltage limits and extra features.
  Apply the installed R part's behavior; identify the edition when comparing tables.
- The supplied `components_m95512r_datasheet.pdf` is identified by hash below.
  It provides a fixed copy for comparison with changing vendor URLs. The installed
  generation's applicability still needs care when importing later specifications.
- ST's AN2014 and TN1259 are listed as additional research below. Their architecture
  and reset material may strengthen the current partial-write and power models.

The MCU flash controller has separate geometry, control and verification rules in
the [target manual/addition](#target-and-instruction-manuals). The external EEPROM's
internal write cycle cannot define the MCU's guest-controlled flash procedure.

## Board and firmware

- [Dmitry Grinberg's Pokéwalker investigation](https://dmitry.gr/?proj=28.+pokewalker&r=05.Projects).
  Primary reverse engineering of the actual product, component identification and
  firmware execution. Particularly useful where board context is absent from chip
  manuals. Distinguish observed hardware from the author's proposed explanations.
- [reverse-pokewalker board notes](https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/doc/Board.md)
  and [photographs](https://github.com/mamba2410/reverse-pokewalker/tree/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics),
  commit `7a409ff625e95457a55832a4e89ebdefa0c7cec6`.
  Component markings, placement, test pads and reported connections. The battery
  investigation specifically uses
  [side A](https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics/sidea-bare-02.jpg)
  and [side B](https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/pics/sideb-bare-01.jpg).
  Photographs do not establish every buried connection. RES2B and VCI remain unidentified
  in the board notes.
- [Associated 64 KiB dump](https://github.com/mamba2410/reverse-pokewalker/blob/7a409ff625e95457a55832a4e89ebdefa0c7cec6/dumps/bin/64k-full-rom.bin).
  Context for the battery-calibration analysis cited in
  [ADC board transfer](research/adc-board-transfer.md). Interpret its contents and
  acquisition context before treating its filename as a memory-map description.
- [Nintendo Pokéwalker operations manual](https://csassets.nintendo.com/noaext/image/private/t_KA_PDF/Pokewalker_Tri?_a=BATCtdAA0).
  Original instructions for connection, battery replacement and user-visible operation.
  Useful system-level expectations; it does not define internal register timing.

### Matching firmware

The reviewed [`lumirth/pw` tree](https://github.com/lumirth/pw/tree/6dc7bc09950078fa3fe0dffa4dae34e9549a99da)
is pinned to `6dc7bc09950078fa3fe0dffa4dae34e9549a99da`. Matching code is strong
evidence of the instructions the retail device successfully executed. Read the full
sequence and its state; comments can have a weaker basis than the accesses themselves.

| Firmware area | Useful evidence |
| --- | --- |
| `src/startup/h8_resetprg.c`, `h8_intprg.c`, `include/startup/iodefine.h` | Startup, vector assignments, memory/register layout and clock setup. |
| `src/application/pw_ssu_init.c`, `pw_accel_bma150.c`, `pw_main.c` | Shared bus wiring, sensor initialization, protected writes and sample acquisition across clock modes. |
| `pw_nt7508.c` | Command/parameter chip-select behavior, plane order, geometry, palette and oscillator initialization. |
| `pw_eeprom_m95512_bus.c`, `pw_eeprom_m95512_io.c`, `pw_storage.c` | WIP polling, write-enable, page writes, mirrors and recovery. These routines concern external EEPROM. |
| `pw_battery.c`, `pw_selftest.c` | Switched ADC acquisition and stored factory calibration. A threshold record does not establish a universal circuit voltage. |
| `pw_player_input.c`, `pw_buzzer.c`, `pw_power.c`, `pw_rtc.c` | Button polarity, buzzer pin use, sleep and stable clock reads. |
| `src/support/lib_common.c` | Clock transitions, watchdog accesses and conditional delay loops. |
| `src/support/ir.c` | SCI setup, transceiver pin polarity, turnaround, timeouts, echo constraints and protocol. The protocol executes in firmware. |
| `pw_home.c`, `pw_pictogram_menu.c`, `pw_friend.c` | Connection eligibility and persistent consequences used to interpret complete retail exchanges. |

Unqualified filenames in the table are in `src/application/`. Exact functions and
line references are linked from the relevant hardware notes. Hardware rules must also
explain custom firmware; reaching a retail branch is evidence of use, not the limit
of the component specification.

## Infrared components

The board sources do not establish a transceiver part number. These manufacturer
documents support comparison of plausible mechanisms, with the limits recorded in
[infrared evidence](research/infrared.md).

| Source | Use and applicability |
| --- | --- |
| [ROHM RPM871-H14](https://media.digikey.com/pdf/data%20sheets/rohm%20pdfs/rpm871-h14.pdf), pp. 4–5 | RX pulse shaping, transmit cutoff and recovery. Candidate mechanism evidence; exact values are not established for the Pokéwalker. |
| [Vishay IrDA circuit/layout application note](https://www.vishay.com/docs/82610/irdatransceiver_referencelayoutscircuitdiagrams.pdf), pp. 7–9 | Shutdown, AC reception and circuit constraints. Useful for interpreting why incident light need not equal a digital RX pulse. |
| [Vishay TFBS4711](https://www.vishay.com/docs/82633/tfbs4711.pdf), pp. 2 and 4 | Echo, startup and turnaround illustrate part-specific behavior. Later revisions include a 2022 change and cannot establish this 2009 board's exact behavior. |

## Emulator design and validation

These sources help choose representations, interfaces and experiments. Pokéwalker
hardware claims still need the applicable component, board or firmware evidence.

| Source group | What was examined and where to find the specific references |
| --- | --- |
| [SameBoy](https://github.com/LIJI32/SameBoy/tree/213a12ce93d66b105a113debd9396306066a7cfc) | Core state, rewind, battery saves, serial callbacks, LCD/audio output and frontend pacing. [State/testing](research/emulator-state-and-testing.md), [linking](research/emulator-linking.md) and [presentation](research/emulator-presentation.md) cite individual files. |
| [mGBA](https://github.com/mgba-emu/mgba/tree/25ca25612eb806ad3a70f3209ccd93890ea0c2c6) | Serialization, savedata, core tests, serial drivers and local link coordination. The same state/linking notes explain the consequences for HachiStep. |
| [melonDS](https://github.com/melonDS-emu/melonDS/tree/906e9ebb27da8c6a715cd7abab4abfe8a8d29427) | Platform transport and frontend execution scheduling, examined in [linking](research/emulator-linking.md). |
| [Pocketwalker](https://github.com/h4lfheart/pocketwalker/tree/2f3b4512a668e3b7c321f213c1c8d5344627e96e) | Existing Pokéwalker emulator's SCI, board and Qt networking boundary. Used for comparison in [linking](research/emulator-linking.md), not as the hardware oracle. |
| [jgenesis](https://github.com/jsgroth/jgenesis/tree/d19ee94f64798946e30d8be7a3dc6f1f9f8732c8) and [binjgb](https://github.com/binji/binjgb/tree/c60e138da5a795ebb55e56b11b7e90024e41112c) | Core dependencies, rendering/audio interfaces and ownership. [Dependencies](research/emulator-dependencies.md) and [presentation](research/emulator-presentation.md) preserve the comparisons. |
| [Dolphin](https://github.com/dolphin-emu/dolphin/tree/ee018d00e60b9eb727489908a8daec5c537f44a8) | State traversal and timing tests, discussed in [state/testing](research/emulator-state-and-testing.md). |
| [Near on cooperative threading](https://archive.ares-emu.net/near.sh/articles/design/cooperative-threading.html) and [serialization](https://archive.ares-emu.net/near.sh/articles/design/cooperative-serialization.html) | Timing and continuation representation. Useful design reasoning rather than a required architecture. |
| [MAME CPU device specification](https://docs.mamedev.org/techspecs/cpu_device.html) | CPU execution and suspension interfaces. |
| [mGBA on emulation accuracy](https://mgba.io/2017/04/30/emulation-accuracy/) | Accuracy claims, software compatibility and the role of tests. |
| [Cycle-stepped 6502](https://floooh.github.io/2019/12/13/cycle-stepped-6502.html), [rs80](https://github.com/cbiffle/rs80), [Gameroy optimization account](https://rodrigodd.github.io/2023/09/02/gameroy-jit.html) | Alternative execution representations and performance tradeoffs considered in the design. Their chips and workloads differ from this target. |
| [Mooneye test suite](https://github.com/Gekkio/mooneye-test-suite/tree/31510e12eea6286d36eea060a6adde755e1067aa), [SameSuite](https://github.com/LIJI32/SameSuite/tree/f15645fb049a47ea235f6d2c9a033e72d8087901), [mGBA suite](https://github.com/mgba-emu/suite/tree/e6942030d25ffe3ba76c72b73a86da073ec857cc), [Dolphin hardware tests](https://github.com/dolphin-emu/hwtests/tree/f28077b139eec18967f60db6ce1e15b182dfeac0) | Independent diagnostic programs and expected observations. Specific examples are discussed in [state/testing](research/emulator-state-and-testing.md). |
| [Gekkio's test-ROM guidance](https://gekkio.fi/blog/2016/game-boy-test-rom-dos-and-donts/), [SingleStepTests 8088](https://github.com/SingleStepTests/8088), [SameBoy 0.15 accuracy changes](https://sameboy.github.io/posts/release-0.15) | Test design, instruction observations and the scope of accuracy improvements. |
| [Pan Docs](https://github.com/gbdev/pandocs), its [reference catalogue](https://github.com/gbdev/pandocs/blob/master/src/References.md), [Gekkio's technical reference](https://github.com/Gekkio/gb-ctr), [SameBoy OAM research](https://github.com/LIJI32/SameBoy/discussions/750) | Organization of established knowledge, sources and unresolved mechanisms. These informed the accuracy-document structure. |

The [BESS specification](https://github.com/LIJI32/SameBoy/blob/213a12ce93d66b105a113debd9396306066a7cfc/BESS.md)
informs the distinction between portable best-effort state and exact continuation.
[Borsh](https://borsh.io/) specifies encoding. The [state contract](SAVE_STATES.md)
defines what HachiStep serializes. Serialization format does not establish hardware
fidelity.

[hachiware](https://github.com/lumirth/hachiware) owns Pokéwalker diagnostics and the
basis of their expected observations. [TESTING](TESTING.md) describes HachiStep's
adapter, checks and interpretation of results.

## Additional research leads

These are useful places to extend the existing research. The identified topics have
been checked against the documents' accessible descriptions or relevant sections.
Detailed comparison with the implementation is still required before adopting a
new hardware claim. Retain useful findings in the existing mechanism note and update
the corresponding accuracy section.

### Located manufacturer material

| Source | Question to investigate |
| --- | --- |
| [Timer B1 counting seconds](https://www.renesas.com/en/document/apn/h838602r-group-application-note-counting-seconds-using-timer-b1), REJ06B0512-0100, March 2005 | Independent reload/counter example. It uses a 38.4 kHz crystal; calculate expectations with the actual configured source rather than importing a one-second result. |
| [AEC PWM output](https://www.renesas.com/en/document/apn/h838602r-group-aecpwm-output), REJ06B0516-0100, March 2005 | Period, low-time and invalid duty/period relationships, plus the P12 pin selection. Useful for reviewing PWM and connected-pin effects. |
| [SSU EEPROM communication with GPIO SCS](https://www.renesas.com/en/document/apn/h838602r-group-application-note-data-communication-eeprom-4-line-bus-communication-mode-scs-signal), REJ06B0506-0100, March 2005 | Transfer completion, explicit chip-select control and pin direction. Its EEPROM procedure is guest software. |
| [Clocked synchronous communication mode of SSU](https://www.renesas.com/en/document/apn/h838602r-group-application-note-clocked-synchronous-communication-mode-ssu) | Another same-target serial configuration; compare receive/holding and interrupt sequences with the current model. |
| [Sensor connection with low supply current using comparator and ADC](https://www.renesas.com/en/document/apn/sensor-connection-low-supply-current-using-comparator-and-ad-converter), REJ06B0644-0100, March 2007 | Comparator hysteresis, low-power sensing and ADC interaction. Its application circuit does not establish Pokéwalker wiring. |
| [Waiting time for clock stabilization](https://www.renesas.com/en/document/apn/setting-waiting-time-cover-clock-stabilization-timing-reset-release-and-setting-wait-states), REJ06B0645-0100, March 2007 | Reset release, oscillator stability and STS waits. The note groups several MCU families; establish the correct group before applying its tables. |
| [ST AN2014](https://www.st.com/resource/en/application_note/cd00042024-how-a-designer-can-make-the-most-of-stmicroelectronics-serial-eeproms-stmicroelectronics.pdf), current index identifies rev. 11 | EEPROM cell/array architecture, SPI selection and power reliability. Useful for evaluating the partial-write model. Compare historical editions and applicability to M95512RP; later process details may differ. |
| [ST TN1259](https://www.st.com/resource/en/technical_note/tn1259-poweron-reset-characterization-for-serial-eeproms-stmicroelectronics.pdf), DocID031357 rev. 1 | POR parameters, SPI characterization procedure and measured-value tables. Check product/process applicability before changing an installed-part threshold or reset rule. |
| [ST serial EEPROM documentation index](https://www.st.com/en/memories/serial-eeprom/documentation.html) | Further vendor notes, including AN626 product numbering and available device models. A source for identifying the installed generation and additional technical material. |

### Unrecovered or unidentified material

| Lead | What it could resolve; what remains to establish |
| --- | --- |
| Bosch ANA016, "In-line offset re-calibration", and the BST-MAS-AN014-01 reference | Earlier BMA150/SMB380 calibration guidance. Search archived Bosch packages and vendor SDK mirrors. Existing searches recovered references to the note, not the note itself. Do not assume it defines all gain or temperature trim fields. |
| Full board netlist and unidentified component markings | Battery sense topology, reset connections, VCI/RES2B and passive values. Existing photographs and firmware constrain the possibilities but do not complete the circuit. |
| Installed IR transceiver identity | Which pulse shaping, turnaround, shutdown, echo and TX protection specifications apply. Compare package/pins and board evidence before importing a candidate part's behavior. |
| Actual LCD glass and piezo characteristics | Optical response, persistence, acoustic resonance and drive sensitivity if a frontend needs a physical presentation model. Digital controller and buzzer timing remain independently reviewable. |
| Historical M95512RP datasheet/process information | Applicability of current family timing, voltage, write-interruption and retention descriptions. The supplied R copy is a fixed starting point. |
| Further target amendments and original-language explanations | Resolve a specific contradiction or missing transition. Search by document number as well as part name, including H8/38602, H8/38602R and H8/38606. A related-family note requires an applicability check. |

Original chip/component documents remain useful even when a particular question is
unresolved. Firmware, vendor code and board evidence can jointly support a strong
inference. Record the resulting mechanism and its basis rather than waiting for a
single document to state the entire answer.

## Supplied images

Firmware was extracted from the user's File Library archive `pw-inputs.zip`:

```
Archive bytes: 248796246
Archive SHA-256: e6400eee35504fea477ae466ed33bcc8c5b5cd313962ad56c351e7c24e1c3460
Member: pw-inputs/firmware/nintendo/pokewalker/retail-48k/pokewalker.bin
ROM bytes: 49152
ROM SHA-256: f9e210a3b74afbbd12c5a66a51cc05cb9fbac986805ff0a3bfb4be6074d15607
Reset vector: 0x02c4
```

The archive was inspected for a 64 KiB EEPROM and did not contain one. The EEPROM used
in the runs was acquired separately from the user's File Library:

```
EEPROM bytes: 65536
EEPROM SHA-256: 9b9d7ac29b3d27de8fed1aca392c91ec559a53c2c22f2a9c860c980895539008
```

The images are unmodified. Retail verification checks their identities before and after
execution against `workloads/retail.json`.


## Supplied document identities

The original PDF bytes are not redistributed. These hashes identify the supplied copies,
including the SSD1854 reference whose presence does not identify the installed LCD.

| Document | SHA-256 |
| --- | --- |
| `components_bma150_datasheet.pdf` | `8e07fb86bce3daaa2c4b9b86558dfcba8ae6c861e4e1696cd8006b022d514eff` |
| `architecture_h8-300h_programming-manual-ade-602-053a.pdf` | `5c79702dcafcba0adacf77b8672bb1358ad5519ebf20d98e1b2f88000a32ed98` |
| `devices_h8-38602r_h8-38606-addition-note.pdf` | `416cd2b732a060b038519b0e38e935bf675a2fb99cd2d1554bc0daaf732ed2bb` |
| `components_ssd1854_datasheet.pdf` | `27a6bf6140ec2ae72898ed1e91526a88592dc6d9c9b2165ee019cc6c99782ff9` |
| `components_m95512r_datasheet.pdf` | `0f91e5ebc32c8eed6f389be81b01a2e90aa94c3a9b118a5fac428cd61af76bab` |
| `devices_h8-38602r_hardware-manual.pdf` | `5029638c5ab3e3448fbadb9dcbe689ff8e74fbd50412e3abd5720f1651a1cc0f` |


## Detailed hardware notes

These notes preserve the interpretation and local model choices behind the accuracy
overview. Their inline references locate individual claims within the catalogue's
documents and code.

| Mechanism | Reference |
| --- | --- |
| CPU fetch, access order, exceptions and admission | [Execution](research/h8-execution.md) |
| Encoding and arithmetic | [Encodings and arithmetic](research/h8-encoding-and-arithmetic.md) |
| Clocks and SCI/IrDA | [Clock and serial rules](research/h8-clock-and-sci.md) |
| Timers, RTC, watchdog, ADC and AEC | [Counter and converter rules](research/h8-counters-and-adc.md) |
| Register bus and GPIO | [Access and pin rules](research/h8-registers-and-gpio.md) |
| SSU and IIC2 | [SSU](research/h8-ssu.md), [IIC2](research/h8-iic2.md) |
| Internal flash | [Flash](research/h8-flash.md) |
| Supply and reset | [Board supply, retention and startup](research/power-and-reset.md) |
| ADC board circuit and comparators | [Battery sensing](research/adc-board-transfer.md), [comparators](research/h8-comparators.md) |
| BMA150 sensor | [Sampling, filtering and register behavior](research/bma150-behavior.md), [I²C](research/bma150-i2c.md) |
| LCD and EEPROM | [Controller and storage behavior](research/lcd-and-eeprom.md) |
| Infrared board behavior | [Optical interface and firmware evidence](research/infrared.md) |


Native save-state semantics are in [SAVE_STATES](SAVE_STATES.md) and
[CPU_STATE](CPU_STATE.md). The [design](DESIGN.md) records the architectural contract.
