# Implementation handoff

## First commands

```sh
python3 tools/check.py --out out/your-check
python3 tools/verify_retail.py --out out/your-retail
cargo run -p hs-core --release --example replay -- \
  local-inputs/pokewalker.bin local-inputs/eeprom.bin
```

All destination directories must be new. Logs remain available on failure.
The standard checks do not require private images. The retail command is
explicitly separate and verifies that input hashes remain unchanged.

## Reading route

Read `machine.rs::run_until` and its inner loop, then `cpu::Cpu::next/complete`,
then the addressed MCU owner. Device state does not depend on a retail function
address. Product output is collected at `Output::event`. Before changing timing,
read the exact-horizon and error semantics in ARCHITECTURE.

The synthetic conformance builder is deliberately separate from Rust and does
not import the decoder. Its tiny emitter only generates the diagnostic forms it
needs; it is not intended to become a general assembler.

## Ordered work

### 1. Close CPU fetch, timing and admission

Files: `cpu/mod.rs`, `cpu/decode.rs`, `machine.rs`, `mcu/mod.rs`,
`conformance/build.py` and `crates/hs-core/tests/kernel.rs`.

The executor already preserves partial physical accesses. Extend that same
continuation rather than adding a whole-instruction backend. Audit each form's
fetch, discarded prefetch, internal delay, access and retirement timing against
the target manual. Make request sampling/admission explicit where a final-state
mask check is insufficient. Close CCR-write deferral, NMI/EEPMOV interruption,
stack quirks, aliasing updates, odd addresses and undefined-form dispositions.

Acceptance: independent form/flag cases, exception bus ordering, interrupted
multi-beat accesses, self-modification before/after fetch, boundary phase sweeps,
and no change in the single-engine invariant. First-word nonpanic enumeration
alone is not the acceptance criterion.

### 2. Replace absolute in-flight appointments at clock transitions

Files: `machine.rs::Pending`, `mcu/clocks.rs`, `control.rs`, `ssu.rs`, `sci.rs`,
`adc.rs` and timer owners.

Store the applicable remaining clock obligation where a source can change during
an operation. Distinguish divider phase, downstream gating and oscillator stop.
The present system-source switch rephases fractional timing; correct this from
the target's source-switch/stabilization rules. Add owner-specific same-time
conflict handling rather than generalizing the current global ordering.

Acceptance: phase sweeps of clock/gate changes inside bus, serial and conversion
operations; whole/chunked/snapshot behavior still agrees; each new rule has a
manual anchor or identified measured basis.

### 3. Complete pin/function and serial modes

Files: `gpio.rs`, `ssu.rs`, `sci.rs`, `machine.rs::resolve_board` and external
serial owners.

Resolve 0xF088. Complete pin priority/open-drain/pull behavior, serial slave and
bidirectional modes, alternate serial selection, SCI stop-bit/error/multiprocessor
cases, active reconfiguration and the actual IR transceiver envelope. Do not
invent a BMA IRQ wire or direct packet-to-register bypass.

Acceptance: physical pin fixtures, simultaneous selects/contention, deselect at
every bit position, output released versus driven high, status clear versus byte
completion, real peer interoperability only when actually measured.

### 4. Complete missing MCU owners

Add concrete `aec.rs`, `iic.rs`, `comparators.rs`, `flash.rs` modules; complete
Timer W capture/buffering and ADC trigger/retention modes. The address authority
currently rejects these operations, so entry points are easy to identify.

Acceptance: owner-specific command/register sequences through guest accesses,
not only direct Rust method calls; reset/gate/race tests; invalid configuration
handling; persistent changes and fetches during internal flash programming.

### 5. Characterize sensor, LCD and supply behavior

Files: `devices/bma150.rs`, `nt7508.rs`, `mcu/adc.rs`, `machine.rs`.

Resolve BMA filter lengths/internal precision, startup history, publication skew,
calibration window/0x1E effects, interrupts, other interfaces and self-test.
Resolve LCD COM mapping/scan latching/column boundary and actual control effects.
Replace the linear battery witness with the board's established circuit/transfer
model. Close supply-to-reset and interrupted-programming outcomes without
inventing atomic all-old/all-new results.

Acceptance: physical observations distinguish the proposed models. Keep fitting
and validation stimuli separate. Until then retain explicit witness labeling.

### 6. Optimize measured work, keeping one model

Current opportunities are visible: individual SSU/Sci/timer output boundaries,
3 kHz BMA updates, repeated decoding, and synchronization around polling. Use
`tools/bench.py --left ... --right ...` with matching workloads. Do not install a
second block runner to make a synthetic loop look faster.

For exact edge-run or signal-law compression, retain one owner advancement
function and test expanded output against the uncompressed local recurrence in
tests. Keep input changes and configuration boundaries inside the validity
contract. No compression currently exists, so there is no hidden fallback to
preserve.

Acceptance: full event histories where available, canonical state, chunk/snapshot
invariance, default build improvement on multiple workload classes, code/table
size and memory cost reported. Endpoint hashes alone are not sufficient proof.

### 7. Frontend and durable session work

The CLI is deterministic replay, not a live GUI. The core's time/input/output API
is ready for a platform adapter. Host audio already renders from drive events.
Live transport must not inject input into the past. A portable checkpoint format
must serialize the complete typed causal state; serializing only RAM and EEPROM
would not be a session save.

## Preserve these boundaries

Firmware decides steps, menu behavior, save repair, time counters and protocol
semantics. The core models the machine, not `pw` functions. Host read-only policy
means separate file output, not suppression of guest writes. No source hash or
retail PC may select behavior. Error paths retain already completed effects.
Unknown physical behavior belongs in explicit research cases, not an expanding
public accuracy-settings menu.

## Handoff quality gates

Keep offline build/tests working, add a narrow regression at the owning boundary,
run the private integration corpus after causal changes, inspect the first
hardware divergence rather than patching a final screenshot, and update STATUS
when a witness becomes established or a new mode is completed. Commit changes in
small reviewable steps. Do not upload private inputs or install network CI merely
because a local check exists.
