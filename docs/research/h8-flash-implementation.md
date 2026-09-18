# H8/38606F internal-flash implementation contract

Research, 2026-09-18. This deepens the flash portion of
[h8-flash-and-iic.md](h8-flash-and-iic.md); it proposes implementation behavior,
with its implementation checkpoint recorded below. Sources are Renesas **REJ09B0152-0300 rev. 3**,
**TN-H8*-A414A/E**, the cached specification update, and public `lumirth/pw`.
Printed hardware-manual pages are 34 below the PDF page number.
The older **TN-H8*-A287A/E** changes the flash endurance qualification's read
voltage range; rev. 3 already includes it. It adds no different pulse or latch
algorithm. ([Update, p. 11][specification-update])

## Implementation checkpoint

The user-mode controller is implemented in `mcu/flash.rs`, integrated into the
ordinary CPU bus, exception/SLEEP notifications and MCU/board reset domains.
Exposure pages reserve their maximum capacity at construction rather than
allocating during guest execution; snapshot clones preserve that capacity.
Normal reads use the binary array directly. Explicit image/peek operations
project a copy to the observation boundary without advancing the guest or
causing protection. The CLI now exports `flash.bin` separately from its input.

The [independent diagnostics](h8-flash-diagnostics.md) supply nine RAM-executed
hachiware cases. Page programming computes retry and strengthening masks;
block erasure verifies rather than assuming a fixed attempt count. Local tests
cover cumulative partial exposure, clock/reset/power effects, projections,
partitioned execution, mid-pulse restoration and allocation after restoration.
Boot-ROM sequencing and per-pulse persistence callbacks remain outside this
checkpoint; no silent boot-mode support is claimed.

Validation: `out/flash-full-check` passes formatting, debug/release/trace tests,
Clippy, Python contracts and all 109 independent diagnostics.
`out/flash-exports-retail` passes all four retail workloads plus partition and
snapshot replay, with no changed execution reports or prior exports; the new
flash exports equal the original firmware hash in every workload. Retail input
files remain unchanged. These runs are software regression evidence.

## Facts that determine the model

The target has 48 KiB at `0000–BFFF`, 128-byte program units, and six erase
blocks: `0000–03FF`, `0400–07FF`, `0800–0BFF`, `0C00–0FFF`, `1000–7FFF`,
`8000–BFFF`. EB4 is **28 KiB**, EB5 **16 KiB**. This replaces the base manual's
16 KiB geometry. ([Target addition, pp. 3–4][addition])

The firmware controls setup, pulse duration and verification separately. A
30 µs program pulse and 10 ms erase pulse are **attempts**, not complete page
and block timings. Table 21.11 gives typical **cumulative** P-high time of
7 ms per page and E-high time of 100 ms per block, excluding verification.
Its maxima are 200 ms and 1,200 ms. The software algorithms limit retries;
there is no hardware retry counter. Chapter 6 says at most 100 erase attempts,
while the electrical table permits 120 subject to the total-time limit. Neither
number belongs in a hidden automatic erase engine.
([§6.4, pp. 109–113][algorithm], [§21.2.8, pp. 409–410][electrical])

Section 6.3.2 explicitly says flash cannot be read during programming/erase.
Treat **any array read**, including instruction and vector fetch, as a protection
trigger during a pulse. Do not infer read-while-write support for other pages
from §6.5.3's shorter wording. RAM and peripheral accesses remain ordinary CPU
accesses. ([§6.3.2, p. 108][user-mode], [§6.5.3, pp. 114–115][protection])

The public `pw` header confirms the register layout and EB5. Searches of its
`src` and `include` C/H files found definitions but no FLMCR/FENR/EBR1 accesses.
Its M95512 operations concern the **external EEPROM** and do not establish an
internal-flash pulse algorithm.
([Pinned header, lines 17–62][pw], [external EEPROM implementation][pw-eeprom])

## One controller, with persistent cell progress

Keep these authoritative facts:

- The nonvolatile array and intermediate cell exposure, where present.
- FLMCR1, FLER, EBR1, PDWND and FLSHE.
- A 128-byte write latch and page-address latch.
- The current electrical mode: idle, program setup/pulse, erase setup/pulse,
  program verify, erase verify, or unavailable; FLER independently inhibits
  further pulses.
- Setup/recovery timestamps; active-pulse target and last synchronized time;
  verify quadword address, pending settling time and previous sensed data.

The mode is derived from controls plus power/protection, rather than a second
copy of writable register bits. Synchronize the old mode **before** each
control write, array access, exception/SLEEP notification, source-clock change,
power transition or snapshot. Apply the event afterwards. A register write may
end one exposure and begin another at the same timestamp.

No per-cell scheduler events are necessary. Pulses cannot produce readable
ordinary array data while active: evaluate accumulated exposure when something
can observe or interrupt it. Changed bytes invalidate decode metadata; fetched
CPU material remains unchanged. Save states retain intermediate exposure and
latches, not just the binary array. Capture must not advance emulated time.

### Concrete nominal partial-progress model

The manuals give pulse algorithms and aggregate timing, not cell distributions.
The following is a deliberately small **local physical approximation**, with
parameters isolated for later measurement. It avoids an atomic page commit and
also avoids the fictitious process of programming bytes in address order.

Represent a cell by bounded exposure `q`, erased at `0`, fully programmed at
`Q`. Use `Q = 100 × duration(7 ms)` in an integer fixed-time unit. During an
eligible program pulse add `100 × elapsed` to cells selected by zero bits in
the write latch; during erase subtract `7 × elapsed` from every cell of the
selected block. Saturate at the endpoints. Thus full-scale program exposure
takes 7 ms and full-scale erase exposure 100 ms. A latch bit of one inhibits
programming; programming cannot turn a stored zero into one.

A concrete deterministic starting distribution uses sixteen cell classes,
`r = ((i * 13) ^ (i >> 4)) & 15`, where `i = address * 8 + bit`:

| Sense operation | Return value |
| --- | --- |
| Program verify | Zero when `q >= Q * (16 + r) / 31`, otherwise one |
| Normal read | Zero when `q >= Q * (16 + r) / 62`, otherwise one |
| Erase verify | One when `q <= Q * r / 256`, otherwise zero |

These thresholds provide different cell completion times and stricter verify
margins. The class function and thresholds are **model constants, not measured
H8 values**. The evidence supporting this form is parallel page programming,
per-bit retry masks, additional strengthening pulses, and separate verification;
the numerical scale comes from Table 21.11. Do not present the inferred
distribution as a calibrated prediction of a particular interrupted device.
([Tables 6.4–6.6, p. 111][program-tables], [electrical table][electrical])

Retain intermediate `q` through pulse endings, page changes, reset and power
loss. Otherwise repeatedly interrupted short pulses never accumulate, and
reset can incorrectly repair a partly written cell. No host RNG, wear counter
or spontaneous charge-decay subsystem is needed for this initial model.

Keep ordinary reads as direct array-byte reads. Populate exposure only for
pages that have been exposed to a pulse; untouched binary endpoint cells are implicit.
A straightforward dense `u64` exposure array costs 8 KiB per affected page,
at most 3 MiB if every page is affected. Reserve that capacity at construction
to avoid allocating during custom firmware execution, without populating it
for the normal loaded-firmware case. An equivalent compressed representation is fine.
Use sufficient fixed-point precision or retain remainders so host call
partitioning cannot change exposure; threshold products need wider integer
intermediates. Do not round each call to microseconds.

Table 21.11 guarantees programming/erasing at 3.0–3.6 V and 0–75 °C; these are
not measured charge-pump cutoff thresholds. Initially retain nominal rates
while powered and supplied by the system oscillator; supply loss stops exposure
immediately. Do not invent a voltage/temperature response curve or make 2.999 V
an automatic flash failure merely from the guarantee boundary. This nominal
extension outside that range is a local modeling choice. ([§21.2.8][electrical])

## Register transitions and write-latch addressing

The documented addresses/masks remain `F020/7F` FLMCR1, `F021/80` read-only
FLER, `F022/80` PDWND, `F023` EBR1, and `F02B/80` FLSHE. All are byte registers.
FLSHE gates access to the other four **registers**, not ordinary array reads.
SWE gates programming controls; SWE clear initializes EBR1. Multiple EBR1 bits
clear the selection. Reserved EBR1 bits 6–7 are readable/writable, with a
software requirement to write zero. ([§6.2][registers], [target EBR1][addition])

Use these concrete local rules for sequences that the prescribed algorithm
does not exercise:

| Event | Controller action |
| --- | --- |
| FLSHE is zero | Gated register reads return zero and writes are ignored. FENR remains accessible. |
| Clear FLSHE during a pulse | Hide registers; do not erase latches, reset registers or stop the pump. |
| Write FLMCR1 with SWE zero | Clear effective programming controls and EBR1; end exposure without rolling it back. |
| Write EBR1 | Preserve a single written bit, including reserved bits; clear on more than one set bit. Reserved-only selections erase nothing. |
| Byte-write array while SWE is set, with no pulse or verify selected | Store at latch index `address & 127`; update page address to `address & !127`. Last write to a slot wins. |
| Write across page boundaries or omit bytes | Retain other latch slots; the last loaded page address wins. Do not recognize or reject a software “128-byte command.” |
| Write array while a pulse is active | Ignore data-loading writes; the active pulse uses its latched page/data. |
| Change EBR1 during E | Settle exposure for the old block, then select the new block for subsequent exposure; setup voltage is already established. |
| Simultaneous P and E, or pulse and verify controls | Retain readable bits but inhibit physical exposure and return unavailable array data. Do not invent an undocumented FLER cause. |
| Simultaneous PV and EV | Retain bits, expose unavailable verify data until a single mode is selected. |

Initialize volatile data latches to `FF` on MCU reset; keep them across ordinary
pulse endings. Invalid combinations suppress exposure only for the conflicting
interval. A later valid combination can continue unless FLER is latched.
Wide CPU writes follow their actual physical lanes and ordering; they must not
be prevalidated and rejected as one atomic operation.

These are address/data-latch behaviors, not firmware-function detection.
The manual specifies only the complete, aligned, consecutive-byte loading
sequence; the partial/cross-page rules above are implementation inferences.
([§6.4.1, p. 109][algorithm])

## Setup, settling and verification

Use real elapsed time, independent of CPU divider changes. Exposure requires
SWE, the relevant setup bit, a single requested pulse, a powered system
oscillator, an accessible flash power domain and no FLER. The on-chip RC source
does not satisfy the documented oscillator requirement. Switching to it stops
exposure; switching back allows exposure after setup settles again.
([§6, p. 99; §6.3.2][user-mode])

| Transition | Required settling |
| --- | --- |
| SWE rises | 1 µs before setup/data loading becomes effective |
| PSU rises | 50 µs before program exposure |
| ESU rises | 100 µs before erase exposure |
| P falls, then PSU falls | 5 µs, then 5 µs before the next sense mode |
| E falls, then ESU falls | 10 µs, then 10 µs |
| PV rises / falls | 4 µs / 2 µs |
| EV rises / falls | 20 µs / 4 µs |
| Verify dummy write | 2 µs before new sensed data |
| SWE falls | 100 µs recovery |

The prescribed pulses are 30 µs for program attempts 1–6, 10 µs additional
pulses, and 200 µs later; erase uses 10 ms attempts. The electrical table permits
28–32, 8–12 and 198–202 µs program intervals and 10–100 ms erase pulses.
These specify guest algorithms and electrical limits, not commands for the
emulator to complete work automatically. ([Figures 6.3–6.4][algorithm],
[Table 21.11][electrical])

**Settling inference:** an early P/E assertion accumulates exposure only after
the applicable setup window; writes before SWE settling do not load data.
Recovery windows exclude new exposure or sense updates. Do not silently delay
the CPU to make an invalid algorithm valid. Missing-clock time does not count
toward setup; retain the gate condition and restart setup on oscillator return.

PV/EV uses a four-byte sense latch. A byte dummy write selects
`address & !3`; after both mode settling and the 2 µs address settling, sample
that quadword with the selected thresholds. Word/longword reads consume the
corresponding lanes. For malformed reads, select lanes by low address bits
from the latched quadword; do not silently retarget the decoder. Any dummy
write value selects the address without changing the array; `FF` is the
documented value. Before settling, retain the previous sense latch, initially
`FFFFFFFF`. Byte reads can select a lane without a special failure.
([§6.4.1 item 7; §6.4.2 item 5][algorithm])

Reads with no settled normal/verify sense path return `FF` per byte. That is
the proposed disconnected/precharged-bus value, not a documented guarantee.
No legal or malformed sequence returns a guest `Unsupported` result.

## Protection, CPU activity and power

On any array read while P/E is physically active, the start of any non-reset
exception, or execution of SLEEP during a pulse: settle exposure, latch FLER,
stop exposure, and retain FLMCR1/EBR1. The offending array read returns
unavailable data; subsequent accesses follow the recovery/verify state.
P/E toggles cannot clear FLER. PV/EV remain usable. Do not hide the fault by
introducing an unconditional hardware interrupt mask; the documented protection
events presuppose exception admission. ([§§6.4.3–6.5.3][protection])

SLEEP then performs its normal mode transition. Reset/subactive/subsleep/watch/
standby initialize FLMCR1, FLMCR2 and EBR1 under §6.5.1, without restoring cell
contents or exposure. Full MCU reset also initializes FENR and FLPWCR.
**Manual conflict resolved locally:** §20.3's retention summary and the phrase
“cleared only by a reset” conflict with §6.5.1's explicit three-register
initialization on those mode transitions. Follow the dedicated hardware
protection paragraph, treating those transitions as flash-controller
initialization; do not grant ordinary writes a FLER-clear mechanism.
([§6.5.1][protection], [§20.3, p. 380][retention])

For `FROMCKSTP=0`, settle and abort exposure, make array reads unavailable,
and apply the same flash-controller initialization: §6.7 explicitly equates it
to standby. Retain FENR/PDWND. Re-enabling the module does not resume an old
pulse. This initialization is an inference from that equivalence, not a
separately enumerated register-reset table. ([§6.7, p. 116][module-stop])

Subactive remains readable with either PDWND value: PDWND selects reduced
versus normal flash power. Wake from reduced power or standby requires 20 µs
flash stabilization even with an external oscillator. Keep that readiness
separate from the CPU's programmed STS wait: if firmware chooses too short a
wait, early flash reads see unavailable data rather than an invented extra CPU
stall. ([§6.6, p. 115][power])

RTS is explicitly prohibited from page loading through P clear and between a
verify dummy write and its read. The CPU manual shows its sequential fetch,
stack read and return-target fetch. Route these actual reads through the same
flash owner. Initially retain latches for a wholly RAM-resident return;
do not manufacture a universal RTS exception or unexplained address corruption.
That choice leaves a precise measurement target if hardware shows a distinct
RTS latch disturbance. ([Figures 6.3–6.4][algorithm],
[H8/300H software manual §2.8, pp. 242–244][cpu-returns])

## Boot-mode boundary

Reset samples TEST/NMI/E7_0 after the specified four-state setup: TEST low,
NMI high selects user mode; TEST low, NMI low, E7_0 high selects boot. Do not
make a later NMI level change switch execution modes. Boot selects the system
oscillator and SCI asynchronous 8N1 without pin inversion. After autobaud `00`
exchange and host `55`, it checks/erases all six blocks, returns `AA` (or `FF`
on erase failure), then receives the length and programming-control program,
echoing bytes. It sends final `AA`, disables SCI RE/TE, retains BRR, drives TXD
high, and branches into the received RAM program. Reset and WDT overflow exit
boot. ([§6.3.1, table 6.2, pp. 105–107][boot])

Autobaud measurement begins approximately 100 states after reset release;
RXD should be high when reset releases. This is a real receive-edge measurement,
not recognition of an already decoded host byte. ([§6.3.1 item 3][boot])

The documented receive aperture is still `FB80–FF7F`: the target addition
expands ordinary RAM to `F780–FF7F` but does not amend that boot aperture.
The table's host prose says length is high byte then low byte; use that order
over its abbreviated “low-order byte and high-order byte” communication label.

Keep boot ownership separate from user-flash decoding, and route its erasure
through this same flash owner. An inferred boot sequencer, if used in the
absence of boot-ROM bytes, must reproduce SCI/pin timing and retained state;
an immediate bulk fill on reset does not satisfy the protocol or interrupted
erase behavior. This task supplies no authentic boot-ROM image or exact
instruction-by-instruction boot timing.

## Focused behavior checks for implementation

Check cumulative short pulses across reset, partial erase across power loss,
retry-data masks, a zero-to-one program attempt, and mid-pulse snapshot/resume.
Exercise late write-latch page changes, multi-bit EBR1 clearing, FLSHE hiding
during a pulse, and an unaffected-page fetch causing FLER. Verify that a RAM
exception/SLEEP abort occurs before later CPU accesses, and that reading after
2 µs settling differs from an early verify read. Compare partitioned and
unpartitioned time advancement; never assert one particular host data structure.

[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=4
[specification-update]: https://www.renesas.com/en/document/tcu/h838602-group-specification-changes#page=12
[registers]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=135
[user-mode]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=142
[algorithm]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=143
[program-tables]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=145
[electrical]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=443
[protection]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=148
[power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=149
[module-stop]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=150
[retention]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=414
[boot]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=139
[cpu-returns]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=258
[pw]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/include/startup/iodefine.h#L17-L62
[pw-eeprom]: https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_eeprom_m95512_io.c#L507-L575
