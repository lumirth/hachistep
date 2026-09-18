# Fault recovery review

Source audit, 2026-09-18. Scope: the current Machine facade, CPU/SSU partial
effects, reset/power paths, and native-state validation. No builds or tests
were executed. The traces below are derived from the reviewed source and
existing fixtures; they are not reported runtime observations.

## Recommendation

Treat a latched core/model fault as terminal for that captured session.
`run_until`, `power_off`, and `power_on` must return the original error before
any mutation or callback, including otherwise redundant power calls. An error
after any of these operations starts mutating state must latch the fault.
Remove the reconnect-time clearing of `Machine.fault`.

Keep observation, persistent-image export, and snapshot capture available.
Recovery is an explicit `restore` of a healthy snapshot, or construction of
a new machine from deliberately selected images and conditions. Restoring a
faulted snapshot keeps it stopped. Existing APIs suffice: no `clear_fault`,
automatic reset, hidden rollback, or recovery executor is needed.

This follows the existing native-state promise that a saved core fault stays
stopped, the distinction between preflight validation and an interrupted
operation, and the design's requirement to preserve actual reset domains.
[Save-state contract][save-doc], [run boundary][machine], [design §§5.7, 10.1,
13.6][design]

## 1. Reconnecting can make an invalid CPU continuation runnable

The reviewed `run_until` checks `fault` first and latches an error from
`run_inner` (`machine.rs:1111–1125`). Immediate power methods bypass that
guard, and `power_on` clears `fault` after setting `connected = true`
(`machine.rs:302–317`). Neither power method necessarily causes MCU reset.
[Machine source][machine]

A literal fixture is already present in `kernel.rs:521–529`: reset vector
0100, then bytes `F8 5A 6A 88 F7 80 57 FF`. It stores 5A in RAM and reaches
an invalid instruction. CPU fetch completion has already recorded the fetched
word, incremented its count, and advanced PC before returning `Error::Decode`;
the phase remains `Fetch` (`cpu/mod.rs:664–687`). [Existing fixture][kernel],
[CPU source][cpu]

Continue that source-derived scenario:

1. Run until the decode error at time `t`; retain the fault snapshot.
2. Call `power_off` at `t`. In this fixture it can disconnect the rail, retaining
   the CPU continuation and capacitor charge.
3. Try `run_until(t + 20 ms)`. It returns the old fault before advancing time,
   so no off-state exposure or capacitor decay occurs.
4. Call `power_on`. This reconnects at the same `t` and clears the fault.
   Zero elapsed absence retains the charged RES node; `update_reset` does not
   invoke `reset_mcu`. No destructive retention deadline has occurred.
5. The CPU still has invalid fetched material in `Fetch`. Once its cold main
   source becomes available, the next admitted action reads another word from
   its advanced PC; it does not restart the failed instruction. In this fixture
   another decode error follows with a larger fetched count.

An even smaller witness needs no resumed execution: encode a snapshot directly
after step 4, then decode it. Capture records `stopped_by_core_fault = false`,
but the CPU validator rejects `FetchInstructionWord` with `Decode::Invalid`
unless the fault flag is set. Thus the API can turn an intentionally stopped,
loadable capture into a nominally healthy capture rejected by its own validator.
[Power charge/retention][power], [reset transition][machine],
[CPU validation, lines 298–306][cpu-state], [native capture/restore][codec]

Timestamped `Input::Power` cannot perform this fault escape because it enters
through guarded `run_until`. The immediate and timestamped forms therefore
disagree despite their documented equivalence. [API reset/power contract][api]

## 2. Partial effects are not a resumable transaction

`complete_cpu` removes the pending bus request before its fallible read/write.
It can then commit a register access and emit a bus event before fallible board
resolution, without reaching `complete_action` (`machine.rs:1017–1107`). A
generic retry cannot distinguish an unperformed access from an already
performed one. [Machine source][machine]

SSU makes the same boundary explicit. `advance` increments `edges` and may
remove its last appointment before `resolve_board`; only subsequent sampling
and `finish_edge` retire the frame (`machine.rs:718–725`, `ssu.rs:412–420,
488–503`). Board serial resolution can return an error for opposing external
drivers. It is therefore reasonable for a terminal capture to preserve a
partially completed edge. [Machine source][machine], [SSU source][ssu]

Native validation deliberately accepts an active SSU with `edges == 16` and
no next appointment only when faulted (`ssu.rs:510–519`, fixture 535–548).
Clearing that latch does not perform missing sampling or retirement. For an
externally clocked fixture, a subsequent selected edge increments the count
past the exact-16 completion condition. This is an independent invariant
witness, not a claim that the CPU fixture above produces an SSU fault.
[SSU validation and edge handling][ssu]

Nor is immediate disconnection a reliable recovery boundary: `power_off`
first calls `settle_boundary`, which can revisit a device boundary that failed
after partial work and before refreshing its appointment. Some faults can
therefore fail again before the rail is disconnected. [Machine source][machine]

## 3. Immediate power errors can leave an unlatched partial change

Unlike `run_until`, both power methods return their fallible transition result
without latching an error. This includes errors during `settle_boundary`, before
the direct rail change. [Machine source, lines 302–326][machine]

A bounded public-API fixture for the transition case is:

1. Construct an ordinary healthy machine and immediately disconnect it at zero.
2. Advance the off machine to `Time::MAX`. Its finite retention deadline occurs
   first; once that completes, absent-supply owners have no running deadlines.
3. Call `power_on`.

`connected` becomes true and `Power::set_rail` changes charge, timestamps, and
rail to 3000 mV. `Power::schedule` then attempts `now + 640 ms` for its RC
crossing bracket and returns `TimeError::Overflow`. `change_rail` has not yet
updated external/MCU supply owners, and the fault is still `None`. The
documented checked-arithmetic failure is legitimate; leaving this partial
transition runnable is not. [Machine source][machine], [Power, lines 75–105][power],
[MCU absent-supply deadline][mcu]

## Minimal lifecycle contract and checks

Distinguish errors by where they occur, not just their enum variant. A reversed
horizon, malformed input timeline, failed native decode, or mismatched restore
is rejected before live mutation and must not poison a healthy session. A
fallible physical transition can already have effects when it reports a time
error; that error is terminal. Existing input and restore fixtures already
establish the nonmutating case. [Input validation][machine],
[kernel validation fixture, lines 475–500][kernel], [restore fixtures][state-tests]

Keep the original terminal error latched. Do not relax the healthy native
validator or reconstruct missing continuations to accommodate clearing it.
The fault flag is operational state, not merely text for a status panel.
Encoded restoration may retain the current generic diagnostic while preserving
the stopped condition. [Native codec][codec], [faulted capture fixture,
lines 107–116][state-tests]

Focused regression witnesses for the recommended rule:

- After the existing RAM-store/decode fixture faults, every run/reset-input/
  immediate power attempt returns the same fault and leaves time, captured
  state, and emitted events unchanged. Exercise redundant calls too.
- Repeat through typed and encoded faulted snapshots. Healthy snapshots still
  restore explicitly and reproduce their original input suffix; failed restore
  leaves the faulted session intact.
- Exercise the off-machine `Time::MAX` reconnect. Its first transition error
  latches; later lifecycle calls do not continue or overwrite that error.
- Keep the existing healthy short-dip and sustained-loss tests: terminal-fault
  handling must not change physical power behavior in a healthy session.

Do not add passive off-time advancement for faulted sessions. The model has
already stopped between effects and cannot certify what future hardware state
follows; elapsed host time does not repair that missing transition. A frontend
can restore a prior checkpoint together with its input/output cursor, or start
a distinct cold session from chosen persistent images. The latter is new
initialization, not exact continuation of the faulted instant. Previously
delivered effects are never silently undone. [Design §§5.7, 13.6][design],
[host cursor ownership][api]

[machine]: ../../crates/hs-core/src/machine.rs
[power]: ../../crates/hs-core/src/power.rs
[cpu]: ../../crates/hs-core/src/cpu/mod.rs
[cpu-state]: ../../crates/hs-core/src/cpu/state.rs
[ssu]: ../../crates/hs-core/src/mcu/ssu.rs
[mcu]: ../../crates/hs-core/src/mcu/mod.rs
[codec]: ../../crates/hs-core/src/machine/state.rs
[kernel]: ../../crates/hs-core/tests/kernel.rs
[state-tests]: ../../crates/hs-core/tests/save_state.rs
[save-doc]: ../SAVE_STATES.md
[api]: ../API.md
[design]: ../DESIGN.md
