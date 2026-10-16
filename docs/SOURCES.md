# Hardware references

These are the sources behind the hardware model. Component manuals describe the
parts, board reverse engineering shows how they connect, and the matching `pw`
reconstruction shows how retail firmware uses them. When those descriptions disagree
or leave a quantity open, the [hardware notes](HARDWARE.md) explain our interpretation.

| Reference | Use |
| --- | --- |
| [H8/38602R hardware manual](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual), REJ09B0152-0300, rev. 3 | CPU restrictions, registers, clocks, peripheral timing, reset, and electrical limits. |
| [H8/38606 addition](https://www.renesas.com/en/document/tcu/addition-h838606-group), TN-H8*-A414A/E | Target memory, package, and flash differences, including 48 KiB flash and 2 KiB RAM. |
| [H8/300H software manual](https://www.renesas.com/en/document/mah/h8300h-series-software-manual), REJ09B0213-0300 | Instruction encodings, flags, ordered accesses, and cycle counts. |
| [H8/38602 specification changes](https://www.renesas.com/en/document/tcu/h838602-group-specification-changes), TN-H8*-A287A/E | Clock, register, electrical, and flash corrections. |
| [Watchdog usage note](https://www.renesas.com/en/document/tcu/h838086r-group-h838076r-group-h838602r-group-watchdog-timer-usage-note-0), TN-H8*-A309B/E | Address-dependent MOV.B write defect. |
| [SCI3 specification change](https://www.renesas.com/us/en/document/tcu/about-sci3-specification-change-0), TN-H8*-A333B/E | Five-bit formats and removal of multiprocessor mode. |
| [IICRST usage](https://www.renesas.com/en/document/tcu/notes-use-iicrst-i2c-bus-interface-2-iic2-and-i2c-bus-interface-3-iic3), TN-MC*-A022A/E | Reset effects and retained registers. |
| [IIC STOP issuance](https://www.renesas.com/us/en/document/tcu/notes-about-issuance-stop-condition-master-transmit-mode-i2c-bus-interface-2iic2-and-i2c-bus), TN-MC*-A023A/E | ACKE/STOP collision. |
| [IIC master receive usage](https://www.renesas.com/en/document/tcu/usage-notes-i2c-bus-interface-2-iic2-master-receive-mode), TN-MC*-A017A/E | Receive-hold release defect. |
| [H8/300H technical Q&A](https://www.renesas.com/us/en/document/apn/technical-qa-h8300h-series-application-note) | Interrupt deferral and STC.W clarification. |
| [RTC operation](https://www.renesas.com/en/document/apn/h838602r-group-rtc-operation), REJ06B0515-0100 | Busy/data timing and interrupt sequencing. |
| [ADC using subclock](https://www.renesas.com/en/document/apn/h838602r-group-application-note-ad-conversion-using-subclock), REJ06B0514-0100 | ADC setup and alternate RTC source description. |
| [Comparator with internal reference](https://www.renesas.com/en/document/apn/h838602r-group-application-note-voltage-comparison-comparator-internal-voltage-reference), REJ06B0511-0100 | Ladder selection and hysteresis. |
| [BMA150 datasheet](https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf), rev. 1.6 | Sensor registers, filtering, serial interfaces, timing, and electrical limits. |
| [BMA150 rev. 1.7](https://datasheet.datasheetarchive.com/originals/library/Datasheets-EDS4/DSAEDA00066689.pdf) | Revised I2C timing and revision history. |
| [Bosch BMA150 driver](https://github.com/rebel1/kernel_2.6.36_nvidia_base/blob/df6ea94f6689d47274182e65c0457878dc50b2dd/drivers/input/misc/bma150.c) | Offset encoding and calibration procedure. |
| [NT7508 V1.0](https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf), July 2008 | Display commands, RAM, scan, PWM/FRC, and electrical controls. |
| [M95512-R datasheet](https://www.st.com/resource/en/datasheet/m95512-r.pdf) | EEPROM commands, protection, page writes, and supply limits. |
| [Pokéwalker investigation](https://dmitry.gr/?proj=28.+pokewalker&r=05.Projects) | Device reverse engineering and component identification. |
| [Board notes and photographs](https://github.com/mamba2410/reverse-pokewalker/tree/7a409ff625e95457a55832a4e89ebdefa0c7cec6) | Component markings, test pads, and connections. |
| [`pw` decompilation](https://github.com/lumirth/pw/tree/6dc7bc09950078fa3fe0dffa4dae34e9549a99da) | Matching retail firmware sequences and data layouts. |

The target addition and applicable errata take precedence over base-part descriptions.
For the reviewed English manuals, add 34 to hardware printed page numbers and 16 to
software printed page numbers to get PDF page ordinals.
