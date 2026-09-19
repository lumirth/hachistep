# M95512 EEPROM

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

The device implements the 64 KiB array, 128-byte page behavior, serial commands,
WEL/WIP, status protection and asynchronous internal writes. Chip-select completion
and power interruption affect the device operation; firmware performs checksums,
mirroring and recovery. Save states retain the pending write as well as the cells.

The board investigation identifies M95512RP. The catalogue includes the R datasheet,
family revisions and vendor application notes. Verify the installed generation when
using later voltage, timing or extra-feature descriptions. A newer family's identification
page is not evidence that this board has one.

## Limits and open questions

The component's write-protect input exists in the model but defaults to released;
the machine does not route a board signal to it. HOLD has no implemented serial-pause
input. Establish whether either pin is controllable on the actual board before adding
a route. Their datasheet presence alone does not make them firmware-accessible.

Interrupted programming uses a deterministic erase/program progression, including
same-value writes and writable status cells. The half-duration phase split and cell
threshold schedule are selected approximations. They permit recovery experiments but
do not predict the exact corrupted bits from a particular physical power failure.
ST's EEPROM architecture and power-on-reset notes are useful further sources for this
model. Wear and long-term retention degradation are not established by this progression.

## M95512: concrete protocol rules

| Behavior | Rule | ST reference |
| --- | --- | --- |
| SPI | Input on rising edges; output on falling edges; deselection floats Q. | [§§3–4, pp. 5–6][ee] |
| WEL | Clear at WRITE/WRSR completion, not start. POR and completed WRDI also clear it. | [§6.3.2, p. 14][ee-status] |
| WRSR | Exactly one status byte; deselect before the next rising edge. Persistent mask `0x8C`; reserved bits read zero. | [§6.4, pp. 15–16][ee-wrsr] |
| Protection | BP protects upper quarter/half/all; SRWD plus low W protects status writes. | [§§5.5, 6.4][ee-wrsr] |
| Page writes | Complete-byte CS rise starts one internal operation. Wrap inside 128 bytes; repeated addresses take the last byte. | [§6.6, p. 18][ee-write] |
| Cells | Erase addressed bytes, then program them. Erased reads 0; programmed reads 1. | [§6.6, p. 18][ee-write] |
| Power | POR resets volatile write state; valid supply must persist through programming. | [§5.1, p. 8][ee-power] |

Overlong WRSR is discarded. Power loss resolves partial cells. Page writes wrap locally
and array reads wrap globally. Completion of an internal write remains independent of
CS, CPU sleep, and MCU-only reset.

RDSR supports continuous polling while busy. Preserve refresh between output bytes; the
exact internal capture instant relative to a coincident completion is not established by
the timing diagram. Preparing an output byte in the serial model does not establish that
hardware latches status at that exact instant. HOLD is a serial pause, not cancellation
of internal programming, but add a board route only if that pin is actually connected to
a controllable signal. Do not import identification-page commands from another M95512
variant.

## A compact default for interrupted programming

ST documents separate erase and program stages but leaves their relative durations and
the order of cell transitions unspecified. The model uses these inferred rules:

1. Before the terminating CS edge, data exists only in the volatile page
   buffer. Power loss discards it without changing cells.
2. At acceptance, retain the addressed-byte mask, original values, targets,
   start time and deadline. Maintain WIP and WEL through programming.
   Treat a page as one operation over its selected cells,
   not 128 serial byte operations.
3. During erase, an affected old byte `O` becomes `O & ~E(t)`, where `E` is a
   monotonic mask of erased cells. Once erase ends, programming proceeds from
   zero as `T & P(t)`, where `T` is the target and `P` is a monotonic completion
   mask. Unaddressed bytes remain untouched.
4. Derive each bit's threshold from its cell address using a fixed deterministic
   rule. Status cells use a separate address key. The half-duration split and
   threshold distribution are selected approximations. The current model has
   no per-unit threshold calibration or random seed. These choices live in
   `WriteCycle`, so evidence can refine them without changing the serial protocol.
5. On supply collapse, evaluate the partial cells, report the resulting
   persistent changes, and cancel the internal operation. On power-up, reload
   those cells and reset volatile protocol state. Successful completion always
   produces the entire requested value. A same-value write still performs an
   erase/program cycle and can be damaged by interruption.

Apply the same inferred mechanism to the writable nonvolatile status cells, restricted
to `0x8C`; leave the array alone during WRSR. Normal status-register visibility changes
at completion; after interrupted programming, cold start loads the resulting persistent
status. Keep the interruption calculation in the EEPROM implementation so machine-level power
handling cannot accidentally bypass it.

This needs no per-cell scheduler events or floating-point simulation. Store the
pending write and compute cell thresholds only when an observation requires them,
especially power loss and completion. A save state preserves the pending write, cells,
buffers and time. The configured write duration defaults to 5 ms; each accepted operation
retains its deadline. Exporting an EEPROM save calculates the cell values at the
current time without committing or ending the pending write. Supply loss reaches the
device's interruption handler, which commits the resulting cells and ends programming.

The firmware makes this behavior useful. Its [page writer][pw-page] polls WIP, issues
WREN, transmits 128 bytes, raises CS, and returns. Its
[mirrored writer][pw-mirror] writes payload/checksum pairs separately.
[WalkStartCommit][pw-commit] sets a recovery marker, copies pages, clears the
marker, then performs more updates; [BootRestore][pw-restore] handles that marker.
Preserve those individual operations and let actual firmware perform recovery.

## Implementation and checks

The [M95512](../../crates/hs-core/src/devices/m95512.rs) owns serial and persistent
state; [cell progression](../../crates/hs-core/src/devices/nv.rs) implements the selected
interruption model. Hachiware's
[EEPROM cases](https://github.com/lumirth/hachiware/blob/main/cases/eeprom.py) check page
wrapping and interrupted operations. [Machine tests](../../crates/hs-core/tests/kernel.rs)
check guest writes reaching the device and distinctions between MCU reset and power loss.
Retail save/recovery scenarios add firmware consequences. Their expectations do not
identify an actual device's damaged cells.

The [source leads](../SOURCES.md#located-manufacturer-material) include ST AN2014 and
TN1259. Compare their process and product applicability to the installed M95512RP
before refining the cell or power-on-reset model.

[ee]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=5
[ee-status]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=14
[ee-wrsr]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=15
[ee-write]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=18
[ee-power]: https://www.st.com/resource/en/datasheet/m95512-r.pdf#page=8
[pw-page]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c#L507-L574
[pw-mirror]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c#L18-L35
[pw-commit]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/startup/h8_resetprg.c#L320-L336
[pw-restore]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_storage.c#L119-L145
