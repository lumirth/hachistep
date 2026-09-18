# Internal-flash owner audit

2026-09-18. Static audit of the current, uncommitted `mcu/flash.rs` and its
`Mcu`/`Machine` integration against the
[implementation contract](h8-flash-implementation.md), Renesas
REJ09B0152-0300 rev. 3, and the H8/38606 addition. No production/test edits,
builds, or new test runs were performed for this audit.

## Result

No concrete flash-owner defect was found in the checked pulse, latch,
protection, clock, reset, or observation paths. The one actionable integration
consequence below concerns the public power-on/reset contract.

| Checked behavior | Implementation trace and conclusion |
| --- | --- |
| Accumulated exposure | `Flash::settle` adds/subtracts elapsed 64.64-time exposure before control changes, clamps cell charge, and keeps it through reset. Each interval is integer-linear until saturation; splitting settlement does not discard a fractional remainder. Changing EBR1 settles the old selection first. The six ranges match the target addition, including the 28 KiB EB4 and 16 KiB EB5. ([Owner, lines 237–315](../../crates/hs-core/src/mcu/flash.rs); [target geometry, pp. 3–4][addition].) |
| Page and verify address latches | Data loads retain untouched slots and select the last written page. A verify dummy write selects a quadword; its readiness includes mode, address, recovery and supply settling. Reads use the sense latch, not the read address's high bits. Mode changes settle or cancel pending sensing before proceeding. These implement the contract's explicit choices for malformed accesses as well as the documented aligned algorithm. ([Owner, lines 198–227, 298–315, 358–381](../../crates/hs-core/src/mcu/flash.rs); [§6.4.1, p. 109][algorithm].) |
| Protection and FLSHE | Array reads during an eligible pulse settle exposure and latch FLER. Exception admission and SLEEP notify the same owner before subsequent CPU accesses; reset follows its separate abort path. Clearing FLSHE only hides the four control registers. FLER blocks subsequent pulses without disabling verification. ([Owner, lines 175–198, 237–243, 316–356](../../crates/hs-core/src/mcu/flash.rs); [CPU integration, lines 835–866](../../crates/hs-core/src/machine.rs); [§6.2.5][register-gate], [§6.5.3][protection].) |
| Clocks, module state and reset | `environment` settles using the old conditions before changing eligibility. It restarts setup on oscillator return and applies the contract's controller initialization on protected power-mode/module transitions. MCU reset preserves array/charge while clearing volatile control/latches. PDWND writes immediately re-evaluate the environment. ([Owner, lines 389–443](../../crates/hs-core/src/mcu/flash.rs); [MCU integration, lines 134–140, 244–269, 604–609](../../crates/hs-core/src/mcu/mod.rs); [§§6.5–6.7][protection].) |
| Host partitioning and inspection | Ordinary `Mcu::sync` does not add a host-dependent flash settlement. `image` and `peek` settle a clone without invoking protected guest reads; `Machine` passes its observation time excluding the unprocessed horizon. Snapshots retain the unsynchronized pulse timestamp plus charge, so cloning need not advance the owner. ([Projection, lines 446–459](../../crates/hs-core/src/mcu/flash.rs); [observation/snapshot, lines 237–266](../../crates/hs-core/src/machine.rs).) |

## Power-on caller contract needs to be explicit

After `power_off` followed by `power_on`, flash moves from stopped to normal
with `ready = now + 20 µs`. `Machine::power_on` installs `Cpu::reset` immediately
and holds it only if the external reset input is already asserted. The CPU's
first action is a two-state vector read. At the default 3.6864 MHz oscillator,
that read happens well before flash readiness and returns `FFFF`, rather than
the stored reset vector. This is a static execution trace through
[power-on, lines 313–336](../../crates/hs-core/src/machine.rs),
[flash readiness, lines 432–433](../../crates/hs-core/src/mcu/flash.rs), and
[reset-vector read, lines 444–449](../../crates/hs-core/src/cpu/mod.rs).

Renesas requires reset to remain asserted through startup oscillator
stabilization and specifies at least 20 µs for flash's return from stopped or
reduced power. Thus returning unavailable data after an early release is
consistent with the electrical model; silently extending every CPU access
would conceal the sequence. ([§6.5.1, p. 114][protection],
[§6.6, p. 115][power].)

Document that `power_on` is a supply transition requiring the caller's reset
sequence, and cover a complete power-off/on/reset-hold/release execution in the
public behavior suite. If the board is instead intended to supply automatic
power-on reset, that belongs in the board/reset owner and should drive this
same reset input. This is separate from the flash pulse checkpoint and from
the already recorded boot-ROM and persistence-callback work.

[addition]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=4
[algorithm]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=143
[register-gate]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=138
[protection]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=148
[power]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=149
