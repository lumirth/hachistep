# H8/38606F internal-flash diagnostics

Independent diagnostic plan, 2026-09-18, for the current starter and
`hachiware`. This complements [the implementation contract](h8-flash-implementation.md).
No expected result below was obtained by running the emulator. **D** denotes
documented behavior; **I** denotes an explicit inference selected in that
contract. Keep those bases in fixture metadata. Programming/erase and interrupted
programming cases must identify their destructive physical-device applicability,
as required by the independent suite's runner contract.

## Verified evidence

[REJ09B0152-0300 rev. 3, §§6.2–6.7][flash] specifies these byte registers,
all initially zero; [§20.1, p. 372][map] specifies two-state accesses:

| Address | Register | Readable bits |
| --- | --- | --- |
| `F020` | FLMCR1 | `40 SWE, 20 ESU, 10 PSU, 08 EV, 04 PV, 02 E, 01 P`; bit 7 zero |
| `F021` | FLMCR2 | Read-only `80 FLER`; other bits zero |
| `F022` | FLPWCR | `80 PDWND`; other bits zero |
| `F023` | EBR1 | One erase selection; bits 6–7 are readable/writable reserved bits |
| `F02B` | FENR | `80 FLSHE`; other bits zero |

FLSHE gates the other four registers, not array reads. SWE=0 inhibits other
FLMCR1 controls and clears EBR1. Multiple EBR1 bits clear its selection. The
[38606 addition, pp. 3–4][addition] changes EB4 to `1000–7FFF` and adds EB5
at `8000–BFFF`; EB0–3 remain four 1-KiB blocks. It retains 128-byte programming.

[§6.4 and Figs. 6.3–6.4][algorithm] separately specify latch loading, pulse
application, and verification. A 30-µs pulse does not promise a finished page.
PV permits word/longword verify reads after an aligned byte dummy write; EV's
prescribed read is longword. Both require 2 µs after that write. The
[electrical table, pp. 409–410][electrical] gives cumulative P/E-high limits,
not autonomous completion deadlines. [TN-H8*-A287A/E, p. 11][update] changes
read-voltage qualification, not these algorithms.

## Fixture body and exact timing primitives

Use a user-mode image, NMI high, I=1, a stable 3.6864-MHz system oscillator,
and nominal 3.3-V supply. These are declared fixture conditions. Copy the body
to RAM before touching flash controls; keep its constants, loops, reporting,
and stack in RAM. An appropriate layout is:

| Range | Use |
| --- | --- |
| `F780–F7FF`, `F800–F87F`, `F880–F8FF` | Wanted, retry, and strengthening data |
| `F900–F97F` | Result bytes |
| `F980–FEFF` | RAM body, including inlined delay loops |
| `FF00–FF6F` | Spare/handler; initial SP=`FF70` |

Place the source body at flash `0400`; copy its padded byte length with
`ER5=0400, ER6=F980, R4=length; EEPMOV.W` (`7BD4 598F`), then
`JMP @F980:24` (`5A00 F980`). Initialize SP with `7A07 0000 FF70`.
The body must fit the stated RAM window. Return through neither a flash
subroutine nor RTS in the prohibited load/pulse or dummy-write/read intervals.
After reporting, clear controls, observe recovery, and use ordinary SLEEP.

Notation below expands to guest instructions, not adapter operations:

- `W(a,v)`: `MOV.B #v,R0L; MOV.B R0L,@a:16`, bytes
  `F8 vv 6A 88 aa aa`; eight φ states with these memory regions.
- `R8(a)`: `MOV.B @a:16,R0L`; store its byte into the next result slot.
- `R32(a)`: `MOV.L @a:16,ER0`; store all four bytes into result RAM.
- `D(N)`, N≥1: `MOV.W #N,R2; loop: DEC.W #1,R2; BNE loop`, bytes
  `79 02 NN NN 1B 52 46 FC`; `4+6N` φ states. Inline it.

These encodings and timings follow the [H8/300H instruction and bus tables][cpu].
For a required delay of 1/2/4/5/10/20/50/100 µs, `D` counts
`1/1/2/3/6/12/31/61` suffice at the declared clock. Pulse timing must include
the eight states of the closing `W`:

| Sequence | P/E-high duration |
| --- | --- |
| `W(F020,51); D(16); W(F020,50)` | 108 states = 29.296875 µs |
| `W(F020,51); D(4); W(F020,50)` | 36 states = 9.765625 µs |
| `W(F020,51); D(121); W(F020,50)` | 738 states = 200.1953125 µs |
| `W(F020,62); D(6142); W(F020,60)` | 36,864 states = 10 ms |

Do not put result logging inside those intervals. The listed program pulses
fit the documented 28–32, 8–12, and 198–202 µs windows.

## Six useful guest cases

### 1. Reset, masks, SWE protection, and register gating

Start immediately after reset. Execute the following rows in order; append
only the specified reads. No pulse is applied.

| Operations | Expected appended bytes | Basis |
| --- | --- | --- |
| `R8(F02B); W(F022,FF); W(F02B,FF); R8(F02B)`; read FLMCR1, FLMCR2, FLPWCR, EBR1 | `00 80 00 00 00 00` | D: reset values; disabled access cannot change PDWND |
| `W(F020,BF); R8(F020); W(F021,FF); R8(F021)` | `00 00` | D: SWE absent; FLER read-only |
| `W(F022,FF); R8(F022); W(F020,C0); R8(F020)` | `80 40` | D: reserved read masks |
| Write/read EBR1 with `20`, `40`, `80`, `21` in turn | `20 40 80 00` | D: target EB5, reserved R/W bits, multiple selection |
| `W(F023,20); W(F02B,00); W(F020,00); W(F023,00); W(F022,00); W(F02B,80)`; read FLMCR1, EBR1, FLPWCR | `40 20 80` | I: gating hides registers without resetting them |
| `W(F020,00)`; read FLMCR1, EBR1 | `00 00` | D: SWE clear |

Complete result: `00800000000000008040204080004020800000`. A sentinel byte at
flash `9000=D3` reads `D3` with FLSHE both clear and set. Do not make the
undocumented value of a *gated register read* necessary to pass this case.

### 2. Verify address/data latch, including a distinguishing early read

Image contents: `9000=12345678`, `9004=9ABCDEF0`.
`W(F02B,80); W(F020,40); D(1); W(F020,44); D(2)` enters settled PV.
Then `W(9000,FF); D(1); R32(9000)` yields `12345678`.
`W(9004,FF); D(1); R32(9004)` yields `9ABCDEF0`.
Clear PV, wait 2 µs, clear SWE, wait 100 µs; ordinary reads yield the same
eight bytes. This **D** case distinguishes byte dummy writes from programming
and preserves the four-byte sense result's ordering.

A separate **I** case probes retained sense data. Prime the latch from `9000`,
then prepare `ER1=9004`, `R0L=FF` and execute the contiguous bytes:

```text
68 98    MOV.B R0L,@ER1
69 12    MOV.W @ER1,R2
69 13    MOV.W @ER1,R3
```

The first word instruction was prefetched before the dummy-write data phase.
Its read completes at +4 states (1.085069 µs), the second at +8 states
(2.170139 µs). The selected old-latch-until-settled model predicts
`R2=1234`, `R3=9ABC`; save them before a delay macro clobbers R2. This is a
precise probe of the chosen boundary, not a claim that Renesas guarantees
early-read data. Reading `9004` without its dummy write similarly belongs in
the inferred-latch case, not the documented companion.

### 3. Page loading is not programming; complete one page through verification

Initialize `8F80–90FF` to `FF`. Desired data is `D[i]=i XOR A5`, i=0..127,
at `9000–907F`. With SWE=0, an ordinary write of `00` at `9000` leaves it
`FF`. Set SWE, wait 1 µs, then transfer the full 128 bytes from RAM in ascending
byte addresses. An ordinary read before PSU/P still gives `FFFFFFFF`; loading
only changed the flash write latch. ([§6.4.1 items 2–5][algorithm], D.)

Use the prescribed retry algorithm, not one fixed pulse followed by a presumed
success. For attempts 1–6 use the 29.296875-µs pulse, plus the 9.765625-µs
strengthening pulse with the computed strengthening data; later attempts use
200.1953125 µs. Each program pulse has PSU setup ≥50 µs, P-clear recovery
≥5 µs, then PSU-clear recovery ≥5 µs. Verify all 32 quadwords using PV setup
≥4 µs and a dummy write plus ≥2 µs at every address; clear PV and wait ≥2 µs.

For each byte, tables 6.4–6.5 give `next_retry = wanted | ~verified` and
`strengthening = previous_retry | verified`. For example wanted `A5`,
previous retry `A5`, sensed `E7` imply retry `BD`, strengthening `E7`.
Stop when verification succeeds, or report failure after 1,000 attempts;
do not derive an expected attempt count from the nominal cell model.

After SWE-clear recovery, expect the full literal XOR pattern in `9000–907F`
and unchanged `FF` neighbors `8FFF/9080`. Copy representative quadwords to
RAM: `9000=A5A4A7A6`, `903C=99989B9A`, `907C=D9D8DBDA`.
Also log successful verify and FLER=`00`. This distinguishes a wrong page base,
short/word-only latch loading, reversed lanes, and programming of padded ones.
An all-`FF` retry latch must leave the completed pattern unchanged: table 6.4
uses ones precisely to inhibit programming of already completed zero bits.

### 4. Target-specific erase boundaries

Use separate fresh images with sentinel zeros at
`0FFF,1000,3FFF,4000,7FFF,8000,BFFF`. The RAM source/loader stays below `1000`.
Run the §6.4.2 loop: SWE setup, EBR1 selection, ESU setup ≥100 µs, a 10-ms
E pulse, E-clear recovery ≥10 µs, ESU-clear recovery ≥10 µs, EV setup ≥20 µs,
and aligned dummy/longword verify reads with ≥2 µs settling. Clear EV and wait
≥4 µs between attempts. Verify the selected block, stop on success, and cap
retries at the flowchart's 100. Clear SWE and wait ≥100 µs before normal reads.

| Selection | Sentinel bytes in the stated order |
| --- | --- |
| EB4=`10` | `00 FF FF FF FF 00 00` |
| EB5=`20` | `00 00 00 00 00 FF FF` |

These **D** expectations detect accidental use of the base part's 12-KiB EB4
and omission/aliasing of EB5. Use verification, not “one erase pulse means FF.”

### 5. An illegal array read latches protection and leaves the CPU running in RAM

Use erased `9000–907F` and load all `FF`, so this first pulse selects no cells.
Set EBR1=`20`, settle PSU, set P (`FLMCR1=51`), and issue a RAM instruction
that data-reads `9000`. Log `FLER,FLMCR1,EBR1`: expect `80 51 20`.
Write zero to FLMCR2 and toggle P; FLER remains `80`. Return a RAM marker
`A5`, demonstrating that this is hardware protection, not a host exception.
The offending read value is not part of the **D** assertion.

After prescribed pulse/setup recovery, enter PV and verify `FFFFFFFF` despite
FLER. A subsequent all-zero programming attempt cannot alter this page until
reset; reuse the bounded pulse budget if testing inhibition physically. A
paired read from a different flash page tests the contract's global
read-unavailability interpretation of [§6.3.2][user-mode].
The protection state and permitted verification are explicit in [§6.5.3][protection].

Do not assume a TRAPA/NMI probe can report normally: its vector fetch is itself
from flash. Test exception-start and SLEEP protection notifications locally,
or observe them externally, rather than creating a fictional RAM vector table.

### 6. Module standby and 20-µs flash wake

With sentinel `9000=D3`, set FLSHE=`80`, PDWND=`80`, SWE=`40`, EBR1=`20`.
From RAM write `CKSTPR1(FFFA)=01`, then `03`. Prepare ER1=`9000` before these
writes. Immediately after re-enable use `MOV.B @ER1,R3L`; after `D(12)` read
again and save both samples. Under the selected unavailable-read/wake rule
expect `FF D3`; an invented CPU stall would incorrectly make the first `D3`.
The early value is **I**; readiness after ≥20 µs follows [§6.6][power].

Afterwards read FLMCR1, FLER, EBR1, FENR, FLPWCR: selected standby-equivalence
initialization predicts `00 00 00 80 80` (**I**, [§6.7][standby]). This catches
accidentally resetting FLSHE/PDWND or resuming an old pulse. The ordinary
subactive control case must remain able to read flash with either PDWND value;
PDWND=0 is reduced-power readable operation, not module standby.

## Two local partial-progress checks, without a cell-layout oracle

1. Apply one identical pulse history with coarse advancement and with stops,
   snapshot/resume, and host partitions inside P-high and verify settling.
   Guest array reads, verify results, control bytes, and later continuation
   must match. Repeat with FLSHE hidden over an interval and ignored gated
   writes; exposure should match the unhidden history. Do not compare private
   exposure arrays or assert one particular class distribution.
2. Compare equal cumulative eligible pulse histories at the same physical
   addresses, inserting a reset between short pulses in one history. Reload
   RAM code/latches and observe setup delays before continuing. Reset clears
   controls; it must not repair partly programmed cells. Include a total history
   that reaches verified completion and compare the guest results. A paired
   late-page latch load—full page, overwritten slot, then a write to another
   page—can be compared against the equivalent final latch/page configuration
   for any common pulse history (**I** for the malformed loading rule).

These comparisons protect cumulative physical effects and the agreed latch
model. They do not turn the proposed 16 cell classes or their thresholds into
independent evidence about the physical Pokéwalker.

[flash]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=135
[map]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=406
[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=4
[algorithm]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=143
[electrical]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=443
[update]: https://www.renesas.com/en/document/tcu/h838602-group-specification-changes#page=12
[cpu]: https://www.renesas.com/en/document/mah/h8300h-series-software-manual#page=207
[user-mode]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=142
[protection]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=148
[power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=149
[standby]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=150
