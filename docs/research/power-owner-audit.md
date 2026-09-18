# Power owner audit

Audited the current power implementation on 2026-09-18 against
[the primary-source startup contract](power-startup-implementation.md).
Scope: rail/reset ownership, source readiness, Machine boundary ordering,
MCU gates, GPIO/serial availability, BMA150, NT7508, and comparators.
Production changes occurred during this review; the final source reread
confirmed both findings below are closed. No further concrete defect was
identified in these paths.

## Closed: interrupted LCD cold qualification became a warm recovery

The original `Nt7508::set_supply` cleared its readiness deadline when digital
supply disappeared, but had already cleared the cold flag on the first
restoration. Consequently, a second, nonzero undervoltage dip during cold
qualification took the warm branch and admitted commands immediately.

Literal trace: after full rail loss, restore 3 V at 0 µs; drop to 1.6 V at
10 µs; restore 3 V at 11 µs. The first restoration requires qualification
through 21 µs, yet the old path accepted commands at 11 µs. The corrected
path remains cold until qualification actually completes and restarts that
qualification after loss: readiness is now 32 µs.

The fix is in `crates/hs-core/src/devices/nt7508.rs:104–146`: the cold branch
retains `cold = true`, and only `at_deadline` clears it. Its owner fixture
at lines 384–399 exercises the second dip and rejects a command inside the
restarted hold.

Novatek requires RESETB during power-up and specifies a 20 µs minimum low
width in the lower supply band plus up to 1 µs completion. The selected
21 µs cold qualification is a board realization of that requirement, not
a documented internal POR timer or measured board delay.
[NT7508, pp.52 and 62](https://www.orientdisplay.com/wp-content/uploads/2022/08/NT7508_V1.0.pdf#page=62)

## Closed: near-zero rail retained sensor configuration indefinitely

Previously, only a sampled rail of exactly zero marked the BMA150 cold.
A 3 V → 1 mV → 3 V trajectory lasting a second could therefore retain
arbitrary volatile configuration and take the warm reacquisition path,
even after the existing shared retention exposure had expired. The MCU
and LCD lost their volatile state at that same physical deadline.

`Machine::devices_at_boundary` now propagates retention loss to the sensor
(`crates/hs-core/src/machine.rs:609–619`).
`Bma150::lose_volatile` marks the next valid recovery cold
(`crates/hs-core/src/devices/bma150.rs:162–180`); that recovery reloads the
nonvolatile image and qualifies serial availability for 3 ms through the
existing `power_on` path at lines 151–160. The owner fixture at lines
605–613 covers a nonzero rail followed by explicit retention loss and
checks the restored serial deadline.

EEPROM-image loading after power-on and the 3 ms startup interval have
manufacturer support. Applying the shared finite retention exposure to
sensor working state is an explicit circuit inference; Bosch does not
specify this project's exposure law or a 10 ms retention time. The fix
propagates the chosen loss event without inventing a new sensor POR
threshold. [BMA150, Table 1 pp.5–6 and §3.3.6 p.21](https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf#page=21)

## Other causal paths checked

- RES release consumes eight actual qualified φ edges. Its hold remains
  distinct from the watchdog's 512-ROSC interval, and completion of either
  preserves the other hold. Cold source availability precedes edge counting;
  watch startup does not hold an otherwise ready main CPU.
  [H8 manual, §19 pp.369–370; §12.3.1 p.208; Table 21.3 p.400](https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual)
- RC and retention deadlines continue while CPU execution is unavailable.
  Supply loss interrupts EEPROM programming before MCU pin withdrawal can
  become an accepted chip-select transition. Unavailable MCU pins lose
  active drive; external serial owners independently enforce their supply
  and readiness gates.
- LCD analog-drive loss remains distinct from loss of digital scanning and
  RAM. Comparator obligations pause through unavailable supply. Warm retained
  sensor recovery and cold image reload remain separate owner transitions.
- Device boundaries precede timestamped inputs and CPU completion. Exclusive
  run horizons do not process the endpoint early, and passive observations
  project copies rather than mutating physical state. Stored physical charge,
  source readiness, and remaining reset obligations support partitioned runs
  and snapshots without restarting timers.

This was a static causal review, including the final production changes and
focused test source. The audit executed no builds or tests; runtime validation
belongs to the implementation task. Board capacitance, first-edge oscillator
waveforms, and below-rated retention remain the explicit selected realizations
documented in the startup contract, not new silicon measurements.
