# Implementation work

The approved design in `DESIGN.md` owns the goals and tradeoffs. This is the
working sequence for completing them; `STATUS.md` records current behavior.

1. Establish fresh native debug/release/trace and retail baselines; separate the
   independent hardware corpus into `hachiware` and retain a thin adapter.
2. Complete CPU fetch/access sequencing and arithmetic edge cases from the
   detailed H8 bus-state tables. Add independent guest observations for the
   corrected mechanisms.
3. Preserve clock sources and pending edge obligations across switching,
   stabilization, gating, serial operation, and reset.
4. Complete hardware owners: serial modes and IIC2, flash, active peripheral
   reconfiguration, sensor algorithms, LCD controller behavior, and power loss.
   Return to manufacturer references, errata, and matching firmware as each
   mechanism is implemented. Use justified inference where evidence requires it.
5. Implement exact native save states with explicit hardware progress semantics,
   candidate validation, and atomic restoration. Keep EEPROM saves separate.
6. Remove redundant execution and scheduling work, measure realistic interactive
   and batch workloads, and verify equivalent observable histories and resource
   guarantees. Finish native/browser integration checks and update usage docs.

Commit and push coherent verified changes throughout. Do not add compatibility
machinery, alternate execution paths, or frontend facilities without a concrete
need.
