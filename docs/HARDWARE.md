# Hardware notes

These notes preserve source interpretations and physical assumptions useful when
bringing firmware to hardware. The [reference list](SOURCES.md) identifies editions.
Hachiware keeps literal diagnostic expectations and their evidence with each guest.

## CPU and register behavior

The H8/38606 addition changes memory geometry without replacing H8/300H instruction
semantics. Use the target's instruction list and timing tables with the software
manual's ordered accesses.

Several manual conflicts need a selected interpretation:

- MOV.L and PUSH.L have identical encodings for ER7 but conflicting word-order rows.
  Use MOV.L's high-word-first sequence. Source/address aliases use the updated register,
  as the instruction description requires. See software manual pp. 127 and 241.
- TRAPA's instruction description and ordered sequence give 14 states. The target
  appendix omits four internal states. See software manual pp. 186 and 243.
- The normal-mode exception stack duplicates CCR in both bytes. STC.W is separate;
  its unspecified odd byte is zero here. See hardware Figure 3.5 and Q&A -037A.
- MOV.L displacement stores accept both selector high-bit values. The consolidated
  table conflicts with the instruction description, while GNU binutils emits the
  high-bit-one form. See software manual pp. 127 and 210 and
  [GNU encodings](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/gas/testsuite/gas/h8300/movlh.s).
- SUBX's sticky Z follows the target flag table. Division by zero retains the destination;
  overflow retains truncated results. The division result-bit choices concern unspecified
  behavior. Unassigned encodings return a host decode error.

The watchdog MOV.B write erratum depends on the instruction's address. Use revision B
of TN-H8*-A309, which supersedes revision A's description.

## Supply and battery sensing

The board's RES capacitance and volatile retention have not been measured. The selected
nominal reset circuit uses 100 nF, the manual's typical 100 kilohm pull-up, and a
0.8 VCC threshold. Its time constant is 10 ms; a discharged capacitor reaches the
threshold after about 16.094 ms. Reset then needs eight actual reference-clock edges.
WDT reset has its separate 512-ROSC hold.

The selected retention law accumulates `max(1500 - Vrail_mV, 0) * dt_ms`, loses volatile
state at 15000 mV ms, and clears that exposure above the 1.5 V retention boundary.
Thus it retains state for less than 10 ms at zero rail. The 1.5 V boundary is documented;
the exposure law and shared component retention are nominal assumptions.

Cold oscillator availability uses manual typical values or bounds: main crystal
300 microseconds at 2.7 V or above, 600 microseconds at 2.2 V or above, and 50 ms below
that; ROSC 15 microseconds; watch crystal 2 seconds at 2.2 V or above and 4 seconds below.
These delays approximate stabilization. First-edge waveforms remain unmeasured.

ADC reference is AVCC. Its nominal transfer is
`clamp(floor(1024 * Vin / AVCC + 0.5), 0, 1023)`, aligned left by six bits.
`pw` drives P84 high, waits, and averages eight channel-7 samples. EEPROM calibration
stores a count, not a voltage reference. The default sense path uses a 600 mV effective
drop from supply, independently of that calibration. The board's actual transfer needs
measurement. `Conditions` and CLI supply/AVCC/drop options expose these quantities.

## Nonvolatile programming

The datasheets specify programming sequences and completion bounds, but do not define
the exact cells left by an interrupted pulse. Flash, EEPROM, and sensor programming
therefore use deterministic partial-cell evolution. Interrupted-byte outcomes are
nominal predictions. Firmware should use its device's documented completion and
verification procedure. Loading a persistent image and restoring a running state have
different purposes, as described in [save states](SAVE_STATES.md).

## Sensor

Bosch's datasheet and calibration code establish offset encoding, but leave numerical
gain and temperature-trim transfer incomplete. The sensor's analog response and noise
are nominal models, rather than a calibration of a particular unit.

The datasheet has conflicting status-bit and wake-duration descriptions. Interpret the
register map together with vendor driver code. `pw` writes protected register `0x1e`
bit 7 during initialization. Preserve that write without assigning it an invented
analog effect. Four-wire SPI is mode 3; writes are address/data pairs, while reads
increment the address. See Bosch section 4.1 and `pw_accel_bma150.c`.

## Display

NT7508 p. 15's generic column-lock sentence conflicts with p. 33's command-specific
wrap rule. Use wrap. E2 uses its explicit reset list on p. 29 and retains RAM.
Pending command parameters survive chip-select transitions, as required by `pw`'s
separate command and parameter writes.

`pw` initializes 64 rows, 3-frame FRC, 9-step PWM, and a clock divisor of 3. With the
nominal 122 kHz oscillator, the frame rate is about 70.602 Hz. Palette widths describe
drive duty, not perceived brightness.

Pixels and COM/SEG outputs represent digital drive. Regulator/follower voltage behavior,
the actual glass's response, and calibrated contrast remain fidelity work. A frontend
chooses its visual response to the exported duty and electronic-volume setting.

## Live reconfiguration and infrared

RTC busy placement at watch ticks 7680 through 8192 and races with running calendar
writes use selected timing interpretations. The manual and application note establish
the busy duration, but do not fix that initial phase. Malformed BCD follows digit
counter widths rather than calendar normalization.

Mid-transfer serial format/clock changes, IIC defect collision windows, comparator
settling, and some timer reconfiguration phases require circuit interpretations where
manuals prescribe only a software sequence. Keep those choices beside the affected
mechanism and refine them with measurements.

Optical inputs model incident levels and timing. Transceiver range, ambient light,
analog sensitivity, and overload recovery require physical characterization. Peer
sessions can verify firmware exchange through the modeled channel; interoperability
with a retail DS needs a physical capture or hardware run.
