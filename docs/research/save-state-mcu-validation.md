# MCU save-state validation inventory

Current-starter source review, 2026-09-18. This is an implementation handoff for
the native codec, covering peripheral owners other than CPU, clocks and flash.
It describes the runtime representation, including unusual states reachable
through accepted guest writes. It does not prescribe normal firmware settings.
Source links below refer to the current working tree; no builds or tests were
run for this review.

## Validation order and scope

Decode into an isolated candidate, validate local shapes and scalar bounds,
then validate clock projections and whole-machine relationships. Only install
the candidate after all checks succeed. Fixed arrays, booleans and enums should
use their typed decoder: no owner below needs a variable-length allocation from
the file. Invalid enum tags are a decoding error.

Validate a value before calling an ordinary helper that assumes it is valid.
In particular, `deadline()` is not universally a safe first validator: RTC,
Timer B1, watchdog and SCI contain unchecked ordinal arithmetic. Use checked
expressions for those preconditions. A failed clock projection is an invalid
candidate, not a reason to advance it until it appears consistent.

For a healthy machine, a scheduled, currently available appointment must not
precede `Machine.now`; equality is valid at the exclusive run horizon. This
bounds the replay loops in RTC, AEC PWM and Timer W before any call to `sync`.
It does **not** apply to every timestamp or every stored wait: ADC settling,
Timer W capture visibility and unpowered comparators have different roles.
Disabled owners can retain old cursors indefinitely. A captured terminal fault
can also retain progress from an interrupted operation; keep its fault latch
and distinguish memory-safety bounds from invariants of a healthy boundary.

Do not validate by calling `read`, `sync`, `set_gate`, `apply_gates` or board
resolution. Those operations consume time, acknowledge flags, change edge
baselines or emit effects. Validate retained gate/mode relationships directly.
The actual load must not manufacture a pin transition.

## ADC

[Source: `Adc`, `sync`, `set_gate`, `advance`](../../crates/hs-core/src/mcu/adc.rs).

- Representation bounds: `mode & !0x7f == 0`, `control & 0x3f == 0x3f`,
  `sample <= 1023`, and `result & 0x3f == 0`. The conversion result is a
  left-justified 10-bit value. No decoded scalar indexes an array.
- Preserve `mode`, `control`, `result`, the held `sample`, conversion `phase`
  and `next`, gate state, `settling`, trigger selection/edge history, the
  two-stage `pipeline`, and `trigger_next`. A disconnected mux or an unsettled
  ADC retains the previous sample, so the sample is causal even while idle.
- `next` uses the selected system/4, system/2, system/1 or watch/1 converter
  clock. Sample and conversion stages consume four and 27 edges. Trigger
  synchronization and settling use CPU/1. Gate closure pauses active work;
  source changes retain unfinished work. Validate owner-specific taps after
  generic wait validation.
- A finished or stopped conversion may retain `phase == Convert`; do not
  require every idle converter to be `Sample`. The ADSF/`next` relationship is
  a healthy-state check, not a reason to erase interrupted progress.
- `settling` is a lazy eligibility latch, excluded from `deadline()`. Its
  timestamp may already be past. See the clock-wait section below.

## RTC

[Source: `Rtc::distance`, `sync`, `next_calendar`, `write`](../../crates/hs-core/src/mcu/rtc.rs).

- Require `phase < 8192` before `distance()` or `sync()`. The normal cycle has
  quarter boundaries and a busy entry at 7680; larger phases can underflow the
  distance calculation. Check `last + distance` before projecting a deadline.
  Reject an already-missed active appointment before entering the calendar
  loop, rather than allowing an arbitrary old cursor to replay years.
- Require `source & !0x7f == 0`, `control1 & !0xf8 == 0`, and data masks
  `[any u8, 0x7f, 0x3f, 0x07]`. **The first byte can contain bit 7:** free
  counter mode uses all eight bits, and switching to calendar mode does not
  sanitize it. Malformed BCD and weekday 7 are accepted guest states. Source
  selectors 8–15 all reach calendar mode.
- A pending `CalendarUpdate` has data masks `[0x7f, 0x7f, 0x3f, 7]`, `pm` in
  `{0, 0x20}`, and flags confined to `0x7c`. Preserve this staged update: guest
  writes during busy can make it differ from the visible calendar, and a mode
  change can retain it. Do not recreate it from current registers.
- All registers, flags, `phase`, consumed `last`, gate and pending update are
  causal. Reset retention is deliberately different from software calendar
  reset. There is no diagnostic field to omit.

## Timer B1 and watchdog

[Timer B1 source](../../crates/hs-core/src/mcu/timer_b1.rs): preserve mode,
counter, reload latch, consumed cursor and enable state. Require the mode's
fixed `0x38` bits. Every `u8` counter/reload value is valid; `256 - load` is
always 1–256, including `load == 255`. Its synchronization is arithmetic, not
an event replay loop. Check `last + (256 - count)` before `deadline()`.

[Watchdog source](../../crates/hs-core/src/mcu/watchdog.rs): require mode's
fixed `0xf0` bits, control1's `0xaa`, control2's `0x57`, and
`rosc_phase < 2048`. Selectors 0–3 alias the ROSC divider; 6 and 7 select no
counter clock and are accepted. Do not reject those register settings.

The watchdog's private ROSC `rosc_last`, `rosc_ticks` and `rosc_phase` are
causal, as are `last`, `seen_overflow`, gate/source qualification and reset
hold. The private divider cannot be recovered from a shared oscillator's
lifetime ordinal. Before synchronization or deadline calculation, check:

- `raw.saturating_sub(rosc_last) + rosc_phase`;
- `rosc_ticks + cycles / 2048`;
- `rosc_last + (256 - count) * 2048 - rosc_phase` for ROSC;
- `last + (256 - count)` for other selected sources.

These runtime additions are not all checked. A valid scalar range alone does
not make an adversarial near-`u64::MAX` ordinal safe. Status already asserted
can suppress the next appointment, so validate arithmetic even when there is
no watchdog deadline.

## AEC and Timer W

[AEC source: `pwm_distance`, `sync`, `write_word`](../../crates/hs-core/src/mcu/aec.rs).

AEC's period, duty and PWM phase may each be **any `u16`**. In particular,
`pwm_phase > period` is reachable after a live period write; period zero and
`duty >= period` have explicit runtime behavior. The arithmetic uses wrapping
distances, and `period + 1` is widened before division. Do not impose a
well-formed-PWM constraint. Registers `edges`, `clock` and `status` accept all
eight bits; `seen` is confined to `0xc0` and `requests` to `0x03`.

Preserve both counters, pin baselines, enable/power gates, per-counter consumed
ordinals, PWM ordinal/phase/**latched output**, pending request bits and `at`.
The PWM output is not always derivable from current phase/period after live
writes. Pending requests are not synonymous with the visible overflow flags.
Validate `at <= now`, checked projections and the active PWM deadline before
the PWM loop. Counter accumulation itself uses widened arithmetic. With the
module disabled, `sync` leaves old consumed cursors in place: that is valid.

[Timer W source: `distance`, `clock_to`, `sync`, `read_word`](../../crates/hs-core/src/mcu/timer_w.rs).

- Fixed bits: mode `0x48`, enable/status `0x70`, each I/O register `0x88`.
  `output` is confined to `0x0f`, and read qualification `seen` to `0x8f`.
  Counter, general registers and saved captures accept every `u16` value.
  Never require the count to be below a comparison value.
- Distances are 1–65536. `clock_to` can replay comparison/overflow events for
  a forged old cursor; validate the active appointment and cursor against
  the clock before invoking it. `input_next` similarly drives a replay loop
  and must use CPU/1 with a valid pending target or paused remainder.
- Preserve `count`, `general`, output and read qualification, gate, `last`,
  `at`, `clear_at`, old capture values/visibility waits, physical input
  baselines, the three-stage synchronizer pipeline and `input_next`.
  `clear_at` is historical: it arbitrates an exact-time CPU count write.
  A past `clear_at` is valid.
- Capture visibility waits use one CPU edge but are not scheduler
  appointments. `read_word` compares their deadline with the owner's `at`,
  while `sync` retires them. Their treatment must differ from `input_next`.
  An old capture value with no visibility wait is inactive scratch; retaining
  it is harmless and avoids an unnecessary second representation.

## Comparators

[Source: `Channel`, `set_supply`, `sync`, `read`](../../crates/hs-core/src/mcu/comparators.rs).

Require positive `response`, `synchronized_at <= now`, and a nonfuture
`unpowered_since` when present. The fixed two-channel arrays do not expose a
decoded array index. Millivolt values are `u16`; their widened threshold
products fit `u32`. Do not add arbitrary voltage restrictions for decoder
safety. The current write path rejects control selection `control & 0x30 ==
0x30`; checking that is a reachability rule, not an arithmetic prerequisite.

Preserve every channel latch: control, result, baseline, armed/read-seen
qualification, flag, target, settling state, due time, event timestamp and
flag-before-event. `event_at` and the previous flag implement same-time read
arbitration and are causal. Keep the sampled supply/reference/input voltages,
gate, response and power-freeze timestamp as well.

An unpowered comparator freezes its due timestamp and adds the off interval
on power return. Thus `due < Machine.now` is valid while unpowered; compare
the outstanding obligation with its freeze epoch and check resumed addition,
not with current wall time. Do not recompute the target from current inputs:
that discards an in-flight inertial response.

## Control and GPIO

[Control source](../../crates/hs-core/src/mcu/control.rs): all table indices
come from masked register fields. Require each `irq_clear_delay <= 2`; larger
values are impossible host progress even though saturating decrement would
not panic. Preserve old IRQ levels, NMI edge/pending latches, feedback-cut
state, mode and `stabilizing_from`. In healthy runtime, the latter names Watch
or Standby. Do not recompute these from current pins.

Register shape checks: sys2 contains fixed `0xe0`; iegr is confined to `0xa3`,
ien1 to `0x87`, ien2/irr2 to `0x45`, irr1 to `0x07`, gate1 to `0x57`, gate2
to `0x7e`, osc to `0xe2`. Sys1 accepts all bits. Those masks describe the
stored latches; avoid further restrictions on accepted combinations.

[GPIO source](../../crates/hs-core/src/mcu/gpio.rs): no stored scalar indexes
an array; routes and register addresses select fixed lanes. The stored masks
are pfcr `0x1f`, pmr `[0x3f, 1, 0x0b]`, first four latch/direction/pull bytes
`[7, 7, 0x1c, 0x0f]`, and open-drain9 `0x0f`. The fifth latch is unused zero;
resolved package-level masks are `[7, 7, 0x1c, 0x0f, 0x3f]`.

Preserve output latches, direction/pull configuration, button/analog/digital
inputs, AEC/clock/SCI/IIC drive projections, floating states and incident
light. `levels` and copied peripheral pad drivers may look derived, but they
are the last resolved board state used by subsequent edge detection. Keep
them in this codec unless a pure reconstruction with identical edge baselines
is established. Calling board resolution during restore is not equivalent.

## SCI, frame and baud generator

[SCI owner](../../crates/hs-core/src/mcu/sci.rs),
[latched format](../../crates/hs-core/src/mcu/sci/frame.rs),
[baud generator](../../crates/hs-core/src/mcu/sci/baud.rs).

Validate every retained `Format` before computing a word, cell count or baud
boundary: data is 5, 7 or 8; stops is 1 or 2; `half_bit` is 16 or 32;
synchronous frames have eight data bits and no parity. Invalid widths can
cause excessive shifts or arithmetic overflow; zero boundary span divides by
zero. The format is latched, so **do not compare it with current SMR/SEMR**.

Progress bounds:

| State | Valid local progress |
| --- | --- |
| TX Blocked | Synchronous character; waiting for admission |
| TX Start | Asynchronous character and scheduled basic-clock ordinal |
| TX Data, synchronous | `cell` 0–7, no asynchronous `next` |
| TX Data, asynchronous | `cell < format.stop()`, `next` present |
| TX Tail | Optional next character; keep its format independent of the outgoing tail's timing |
| RX synchronous | `position` 0–7, `next` absent |
| RX asynchronous | `position` from -2 through `data + parity`, `next` present |

RX positions -2/-1 are real start qualification stages. A smaller negative
position can fall into a data-bit shift with an invalid shift count. TX
`cell == 255` can overflow its increment. Preserve character words and receive
data/parity/error latches; do not reconstruct them from TDR/RDR. Receive error
bits are confined to `0x18`.

Baud `remaining` is 1–256. It can exceed the **current** BRR+1 after a live BRR
change, so that tighter check is incorrect. Retain the selected tap, consumed
source ordinal, remaining reload work, basic half-clock ordinal, polarity,
last edge time, running gate and external-clock selection. Valid taps are
system/1, /16, /64 and watch/1; selection and unfinished reload work are
separate. `last <= now` is a timestamp bound, not a requirement that an idle
baud generator's last edge be recent.

Several SCI helpers add to the basic half-clock ordinal without checked
arithmetic: next falling edge, cell boundary, TX mark/start/tail, IR pulse
start/end and RX sample positions. Preflight those exact additions before
deadline calculation. The largest one-step startup path is at most a boundary
round-up plus a 12-cell frame at 32 halves/cell (less than 416 halves); the
stored IR `bit_start + 13` needs its own check. Basic-clock appointments must
not precede the current basic ordinal when they are active. Preserve stale
historical `bit_start` separately rather than treating it as an appointment.

Register shapes: SPCR fixed `0xc0` with writable `0x13`; SCR confined to
`0xf7`, SEMR to `0x08`, IRCR to `0xf0`, SSR and read qualification to `0xfc`.
SMR, BRR, TDR and RDR accept all byte values. Retain holding buffer, TX/RX
variants, UART/IR/SCK levels, pulse obligations, external input levels,
synchronous-edge obligation and mux-glitch timestamp. **Only `transmitted`
and `received` are diagnostic counters to omit.**

## SSU and IIC2

[SSU source: `advance`, `shift_edge`, `sample`, `finish_edge`](../../crates/hs-core/src/mcu/ssu.rs).

Require `edges <= 16`, `shifted <= 8`, `sampled <= 8`. A healthy scheduled Edge
must have `edges < 16`; finishing the 16th edge clears the transfer. Do not
apply that condition to a pending Load, which can retain counters from the
previous transfer. A terminal fault between producing the final edge and
finishing board sampling also needs separate consideration. The output bit
helper subtracts its index from 7; live edge progress must keep its callers in
0–7. Do not derive sample/shift counts from edge parity: CPHA and related
register bits can change during a transfer.

High has fixed bit `0x08`, clear bit `0x10` and writable mask `0xe7`; low is
confined to `0x58`, mode `0xe7`, enable `0xef`, status/seen `0x4f`. Load waits
use CPU/1; edge waits use the selected system divider or subclock. Preserve
all shift/data/status/qualification latches, selection state, clock and MOSI
levels, sampled external clock, gate, phase and wait. `msb_first` is latched
at load and need not match the current mode. Omit only `transmitted`/`received`
diagnostic counts.

[IIC2 source: `HalfWait`, `Frame`, `rising`, `falling`](../../crates/hs-core/src/mcu/iic.rs).

Require `count <= 7`, `mode & !0xc0 == 0`; in `Frame::Data`, remaining is
1–8, and in Ack/Done it is zero. The data rising-edge path decrements
`remaining` without a checked subtraction. Other register bytes are accepted
as stored; period and monitor table indices are masked. Live mode/control
writes can disagree with a retained frame, and a condition sequence can
coexist with a frame. Do not reject those combinations merely because normal
firmware avoids them.

`HalfWait.count` has two different meanings: absolute system-half-edge target
while running, remaining half edges while paused. Preserve the tag. Resume
adds the remaining work to the then-current half clock with checked arithmetic.
Filter/setup waits use system/1. Clock/monitor waits use half edges; validate
their projections only after the shared clock is valid. Do not bound a pending
duration from current control bits alone, because selection can change after
the obligation was issued.

Preserve raw, filter-latch and filtered line histories, open-drain release
commands, all waits, synchronized hold, start/stop condition and command,
frame, selected/completed/hold state, receive admission/stopping, eighth-fall
time and stale-release latch. These last fields implement timing/erratum
behavior; an old eighth-fall timestamp is not an overdue appointment. There
are no diagnostic counters or freely disposable cached fields here.

## Expired lazy waits and clock epoch reconstruction

[ClockWait projection](../../crates/hs-core/src/mcu/clocks.rs),
[machine boundaries and observation time](../../crates/hs-core/src/machine.rs),
[MCU synchronization/gating](../../crates/hs-core/src/mcu/mod.rs).

A reachable sequence is: issue ADC settling or Timer W capture visibility,
then execute only RAM/flash accesses past its due time. Neither lazy wait is
in the device scheduler, and ordinary memory completion does not synchronize
the MCU. A snapshot can therefore contain `Running { target, ... }` whose
deadline is earlier than the snapshot time. With the same retained clock
epoch, `Clocks::edge(target)` still reconstructs that past timestamp correctly.

There is a separate danger if the source has been rebased beyond that target:
`edge` then fails its checked subtraction. The current Machine integration
settles old rules before the relevant transitions: SFR access, sleep entry,
input/rail changes, explicit power changes, startup completion and wake
completion synchronize the MCU; reset replaces ADC/Timer W state. Both lazy
owners now retire expired waits in that synchronization. **This review did
not identify a healthy current Machine sequence that retains an expired lazy
CPU-edge target across such a rebase.** The expired-without-rebase sequence
above is real and must be accepted by validation.

Recommended codec rule: omit the timestamp cache/revision for pending edge
work, validate by projection without requiring lazy deadlines to be future,
and retain the causal clock epoch. If canonicalizing an expired lazy latch,
do so on the captured copy with an owner-local operation, using the machine's
already-observed time (`max(now - 1, last_effect)`), without calling global
`sync`. Do not silently consume an effect exactly at an unprocessed horizon.
ADC settling can become `None` once its eligibility time has been observed.
Timer W visibility can likewise be retired once observation is past its due
time; its `at`-based read behavior must be considered when doing so.

If a future owner intentionally retains a completed obligation across source
rebasing and still needs its completion timestamp, encode `Ready(actual_due)`
before the old epoch is discarded. That timestamp is then causal history,
not a reconstructible projection cache. Do not make load guess it from the
new frequency, trust an arbitrarily stale cached timestamp, or advance the
guest to repair the state.
