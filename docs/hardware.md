# Hardware interpretations

Sometimes two descriptions of a part disagree. Sometimes a manual gives a range
where a running model needs a value. These notes keep the reasoning behind those
choices, so an unexpected result gives us somewhere useful to start investigating.
The [references](sources.md) identify the source editions.

## CPU and register behavior

The H8/38606 addition changes memory geometry while retaining H8/300H instruction
semantics. The target instruction list and timing tables apply alongside the software
manual's ordered accesses.

| Question | Selected behavior and basis |
| --- | --- |
| MOV.L and PUSH.L word order with ER7 | The encodings match but the ordered-access rows conflict. The model follows MOV.L's high-word-first sequence. Source/address aliases use the updated register, as required by the instruction description. Software manual pp. 127 and 241. |
| TRAPA duration | The model uses 14 states from the instruction description and ordered sequence. The target appendix omits four internal states. Software manual pp. 186 and 243. |
| Exception-stack CCR bytes | Normal-mode exception stacking duplicates CCR in both bytes, following hardware Figure 3.5. STC.W supplies zero for the unspecified odd byte. See Q&A -037A. |
| MOV.L displacement-store selector | Both high-bit values are accepted. The instruction description conflicts with the consolidated table; GNU binutils emits the high-bit-one form. Software manual pp. 127 and 210, and [GNU encodings](https://gnu.googlesource.com/binutils-gdb/+/8ea833b706790cbf50ef3b46028a9ea0d8ecd462/gas/testsuite/gas/h8300/movlh.s). |
| SUBX flags | Sticky Z follows the target flag table. |
| Division result bits | Division by zero retains the destination; overflow retains truncated results. These choices define the model's result for unspecified behavior. |

Unassigned instruction encodings return a host decode error. The watchdog MOV.B
write erratum depends on the instruction's address; revision B of TN-H8*-A309
supersedes revision A's description.

## Supply and battery sensing

The nominal reset circuit uses 100 nF, the manual's typical 100 kilohm pull-up, and a
0.8 VCC threshold. That gives a 10 ms time constant and about 16.094 ms to reach the
threshold from discharge. Reset release then takes eight actual reference-clock edges.
The watchdog has its separate 512-ROSC reset hold. The capacitance and threshold are
chosen board parameters.

A short power cut and a long one can leave different amounts of state intact.
The documented RAM retention boundary is 1.5 V. Below it, the model accumulates
`max(1500 - Vrail_mV, 0) * dt_ms` and loses volatile state at 15000 mV ms. It clears
that exposure above the boundary. At zero rail, state survives for less than 10 ms.
The exposure law and shared component retention are nominal approximations.

Cold oscillator availability uses manual typical values or bounds:

| Oscillator | Delay |
| --- | --- |
| Main crystal | 300 microseconds at 2.7 V or above; 600 microseconds at 2.2 V or above; 50 ms below that. |
| ROSC | 15 microseconds. |
| Watch crystal | 2 seconds at 2.2 V or above; 4 seconds below. |

These intervals model stabilization before clock availability.

ADC reference is AVCC. The nominal conversion is
`clamp(floor(1024 * Vin / AVCC + 0.5), 0, 1023)`, aligned left by six bits.
The `pw` firmware drives P84 high, waits, and averages eight channel-7 samples.
EEPROM calibration stores a count. The battery-sense path uses a nominal 600 mV
effective drop from supply, independently of that calibration. Use `Conditions`
or the CLI's supply, AVCC, and drop options to vary these quantities.

## Nonvolatile programming

The datasheets describe how to program each part and when programming should finish.
They don't determine the exact bytes left by an interrupted pulse. For flash, EEPROM,
and sensor programming, we use deterministic partial-cell evolution to give an
interruption a repeatable result. Those particular bytes are a nominal prediction;
successful programming still follows the documented completion and verification
procedures.

A [save state](api.md#save-and-resume) preserves progress through a pulse. Persistent
image exports preserve the bytes reached at the observation time.

## Sensor

Bosch's datasheet and calibration code establish offset encoding. Gain, temperature
trim, analog response, and noise use nominal models where the sources leave their
numerical transfer incomplete.

For conflicting status-bit and wake-duration descriptions, interpret the register map
together with vendor driver code. The `pw` firmware writes protected register `0x1e`
bit 7 during initialization. The model retains that write; its analog effect is
unspecified in the available sources.

Four-wire SPI uses mode 3. Writes are address/data pairs; reads increment the address.
See Bosch section 4.1 and `pw_accel_bma150.c` in the matching reconstruction.

## Display

NT7508 p. 33's command-specific column-wrap rule governs the conflicting generic
column-lock sentence on p. 15. E2 follows its explicit reset list on p. 29 and retains
RAM. Pending command parameters survive chip-select transitions, consistent with the
`pw` firmware's separate command and parameter writes.

The firmware initializes 64 rows, 3-frame FRC, 9-step PWM, and a clock divisor of 3.
With the nominal 122 kHz oscillator, that gives about 70.602 Hz. Palette widths
specify drive duty.

`display()` returns normalized PWM/FRC duty. `display_drive()` supplies nominal V0-V4
and electrode voltages in microvolts, using the equations and tables on pp. 10, 25-28,
and 42-50. The board rail feeds VCI and VDDA. Temperature compensation uses 1.4 V at
25 C; OTP test mode applies the signed contrast offset in EV steps. The board has
internal LCD supplies and no OTP programming voltage.

These voltage outputs describe steady state. A disabled stage or insufficient pump
headroom leaves a level unavailable. The pump must exceed temperature-compensated V0
by 3 V. Turning those electrical values into an image requires the frontend's model
of the panel's optical response.

## Reconfiguration and infrared

RTC busy placement at watch ticks 7680 through 8192 and races with running calendar
writes use selected timing interpretations. The manual and application note establish
the busy duration while leaving the initial phase open. Malformed BCD follows digit
counter widths.

Mid-transfer serial changes, IIC defect collision windows, comparator settling, and
some timer reconfiguration phases use circuit interpretations where manuals specify
only a software sequence. Their reasoning belongs beside each affected mechanism.

Infrared input represents incident optical levels at specified times. The host defines
the channel between devices and delivers those levels through the
[connection API](api.md#connect-devices).
