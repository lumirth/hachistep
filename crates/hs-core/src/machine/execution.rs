//! Run the CPU with the board as its bus until the next device, input or
//! caller boundary.
//!
//! Every access completes inside the interval at its own time. Memory is
//! served directly; an owner register commits one physical transaction that
//! synchronizes its owners, settles the board and re-derives the interval's
//! boundary, clock window and offered interrupt before the CPU continues. Only
//! an access that would complete at or after the boundary is left pending.
use super::serial::{SerialDevices, ShiftNet, Wires};
use super::*;
use crate::cpu::{
    direct::{DirectRun, Memory, Register, RegisterBus, REGISTER_STATES},
    execution::{Bus, Exit, IntervalBus, Mark, Stop},
    WriteOrigin,
};
use crate::mcu::{
    bus::{self, Access, Local, Registers, Target},
    clocks::{Clocks, CpuCursor, CpuWindow},
    flash::NormalRead,
    ssu::{Instant, Ssu},
};
use core::ops::ControlFlow;

// Output can shorten the horizon; owners finish the current timestamp first.
struct Delivery<'a> {
    output: &'a mut dyn Output,
    end: Time,
    stopped: bool,
}
impl Output for Delivery<'_> {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        let flow = self.output.event(event);
        if flow.is_break() {
            self.stopped = true;
            self.end = self.end.min(Time::from_raw(event.time().raw() + 1));
        }
        flow
    }
    fn exact_stops(&self) -> bool {
        self.output.exact_stops()
    }
}

impl Machine {
    /// Advance through effects before `end`, returning earlier if output requests it.
    /// Inputs at the returned exclusive horizon remain pending.
    pub fn run_until(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
    ) -> Result<RunResult, Error> {
        self.check_fault()?;
        self.board.validate_inputs(end, inputs)?;
        let result = self.run_inner(end, inputs, out);
        self.latch_error(result)
    }
    fn run_inner(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
    ) -> Result<RunResult, Error> {
        let late = !out.exact_stops();
        if self.board.late_output.0 != late {
            // The SSU's and the SCI's need for appointments changes
            // (`serial_lazy`, `Sci::set_late`).
            self.board.late_output.0 = late;
            let board = &mut self.board;
            board.mcu.sci.set_late(late, board.now, &board.mcu.clocks)?;
            self.board.changed_peripherals |= schedule::SSU | schedule::SCI;
            self.board.refresh_peripherals(0)?;
        }
        let mut out = Delivery {
            output: out,
            end,
            stopped: false,
        };
        let mut clock = CpuCursor::new(&self.board.mcu.clocks);
        let mut consumed = 0;
        while self.board.now < out.end {
            // Settle simultaneous device and input causes before the CPU access.
            if self.board.next_devices == Some(self.board.now) {
                self.board
                    .devices_at_boundary(Some(&mut self.cpu), &mut out)?;
            }
            if consumed < inputs.len() && inputs[consumed].at == self.board.now {
                self.board.sync_serial(&mut out)?;
                let start = consumed;
                while consumed < inputs.len() && inputs[consumed].at == self.board.now {
                    consumed += 1;
                }
                self.board
                    .apply_batch(&mut self.cpu, &inputs[start..consumed], &mut out)?;
            }
            self.board.queue_light(inputs, &mut consumed, out.end)?;
            self.queue_cpu(&mut clock, &mut out)?;
            self.advance_cpu(inputs, &mut consumed, &mut clock, &mut out)?;
            // Inputs served inside the interval are consumed.
            let input = inputs.get(consumed).map(|i| i.at);
            let boundary = self.board.execution_boundary(out.end, input)?;
            if boundary <= self.board.now {
                // Output from late serial effects can stop the run behind
                // the CPU (`Output`); it ends where the CPU is.
                if out.stopped {
                    break;
                }
                return Err(Error::Internal("non-advancing event loop"));
            }
            self.board.now = boundary;
        }
        self.board.sync_serial_before(self.board.now, &mut out)?;
        self.board.sensor.sync_local_until(self.board.now, false)?;
        // Every infrared change and light edge before the end is performed;
        // later light edges stay with the caller.
        self.board.report_optical_before(self.board.now, &mut out)?;
        consumed -= self.board.mcu.sci.unqueue_light();
        Ok(RunResult {
            now: self.board.now,
            inputs_consumed: consumed,
            reason: if out.stopped {
                StopReason::Output
            } else {
                StopReason::Horizon
            },
            retired: self.cpu.retired,
        })
    }
    /// Offer the stopped CPU its next operation at `now`: wake, sleep, or the
    /// first access of its work, which becomes the pending access.
    fn queue_cpu(&mut self, clock: &mut CpuCursor, out: &mut dyn Output) -> Result<(), Error> {
        let board = &mut self.board;
        let cpu = &mut self.cpu;
        if board.pending.is_some()
            || board.reset_asserted
            || board.reset_release.is_some()
            || board.watchdog_reset.is_some()
            || !board.power.mcu()
            || (!board.mcu.clocks.available(Tap::cpu()) && !cpu.sleeping())
        {
            return Ok(());
        }
        if let Some(resume) = board.resume_after {
            if board.resume_deadline()? != Some(board.now) {
                return Ok(());
            }
            board.sync_serial(out)?;
            board.resume_after = None;
            match resume {
                Resume::Sleep(_) => board.enter_sleep(cpu, out)?,
                Resume::Wake { direct, .. } => {
                    if board.mcu.sync(board.now, out)? {
                        board.reset_mcu(cpu, true, out)?;
                    } else {
                        board.mcu.control.stabilizing_from = None;
                        board.mcu.apply_gates(board.now, out)?;
                        board.resolve_board(out)?;
                        board.refresh_deadline()?;
                        if direct {
                            cpu.direct_transition()?;
                        }
                    }
                }
            }
            if board.resume_after.is_some() {
                return Ok(());
            }
        }
        if cpu.sleeping() && board.mcu.control.sleeping() {
            let irq = board.mcu.interrupt();
            if irq.is_none() || (irq != Some(7) && cpu.registers.ccr & I != 0) {
                return Ok(());
            }
            board.sync_serial(out)?;
            if board.mcu.sync(board.now, out)? {
                return board.reset_mcu(cpu, true, out);
            }
            let wait = board.mcu.control.wake(board.now, &mut board.mcu.clocks)?;
            board.mcu.apply_gates(board.now, out)?;
            board.resolve_board(out)?;
            board.refresh_deadline()?;
            if let Some(wait) = wait {
                board.resume_after = Some(Resume::Wake {
                    wait,
                    direct: false,
                });
                return Ok(());
            }
        }
        loop {
            let request = match cpu.request(board.mcu.interrupt()) {
                Ok(request) => request,
                Err(error) => {
                    board.sync_serial(out)?;
                    return Err(error);
                }
            };
            if request.admission {
                board.mcu.instruction_boundary();
            }
            if let Some(vector) = request.exception {
                board.accept_exception(vector, out)?;
            }
            if request.action != Action::Sleep {
                let (address, width, write) = match request.action {
                    Action::Read { address, width, .. } => (address, width, false),
                    Action::Write { address, width, .. } => (address, width, true),
                    _ => (0, Width::Byte, false),
                };
                let (states, split) = match request.action {
                    Action::Idle(states) => (u64::from(states), false),
                    _ => {
                        let access = Mcu::classify(address, width, write);
                        (access.states(), split_word(access))
                    }
                };
                board.pending = Some(Pending {
                    action: request.action,
                    wait: clock.after(board.now, states, &board.mcu.clocks)?,
                    split,
                    lane: 0,
                    high: 0,
                });
                return Ok(());
            }
            if board.mcu.control.sleeping() {
                return Ok(());
            }
            board.sync_serial(out)?;
            board.mcu.flash.protect(board.now, out)?;
            if board.mcu.sync(board.now, out)? {
                board.reset_mcu(cpu, true, out)?;
                continue;
            }
            if board.mcu.control.sys2 & 8 != 0 {
                board.resume_after = Some(Resume::Sleep(ClockWait::after(
                    board.now,
                    1,
                    Tap::cpu(),
                    &board.mcu.clocks,
                )?));
            } else {
                board.enter_sleep(cpu, out)?;
            }
            return Ok(());
        }
    }
    /// Run the CPU from its pending access through every access that completes
    /// before the execution boundary.
    fn advance_cpu(
        &mut self,
        inputs: &[TimedInput],
        consumed: &mut usize,
        clock: &mut CpuCursor,
        out: &mut Delivery<'_>,
    ) -> Result<(), Error> {
        loop {
            let input = inputs.get(*consumed).map(|i| i.at);
            let Some(pending) = self.board.pending else {
                return Ok(());
            };
            let Some(at) = pending.wait.deadline(&self.board.mcu.clocks)? else {
                return Ok(());
            };
            let boundary = self.board.execution_boundary(out.end, input)?;
            if at >= boundary {
                return Ok(());
            }
            self.board.pending = None;
            let window = clock.window(at, boundary, &self.board.mcu.clocks)?;
            let mut bus = Execution::new(
                &mut self.board,
                out,
                clock,
                window,
                boundary,
                inputs,
                &mut *consumed,
                pending,
                at,
                self.cpu.instruction_pc(),
            )?;
            let result = self.cpu.run_interval(&mut bus);
            let suspended = bus.finish()?;
            match result {
                Ok(Exit::Horizon(action)) => {
                    let (split, high) = match suspended {
                        Some((_, high)) => (true, Some(high)),
                        None => (pending_split(action), None),
                    };
                    self.board.pending = Some(Pending {
                        action,
                        wait: clock.wait(),
                        split,
                        lane: u8::from(high.is_some()),
                        high: high.unwrap_or(0),
                    });
                    return Ok(());
                }
                Ok(Exit::Fence) => {
                    if let Some((action, high)) = suspended {
                        // The first byte lane stopped the CPU clock; the second
                        // waits for the clock to run again.
                        let (Action::Read { address, .. } | Action::Write { address, .. }) = action
                        else {
                            return Err(Error::Internal("split lane without an access"));
                        };
                        self.board.pending = Some(Pending {
                            action,
                            wait: clock.after(
                                self.board.now,
                                Mcu::access_states((address & !1).wrapping_add(1), Width::Byte),
                                &self.board.mcu.clocks,
                            )?,
                            split: true,
                            lane: 1,
                            high,
                        });
                    } else {
                        self.queue_cpu(clock, out)?;
                    }
                }
                Ok(Exit::Sleep) => self.queue_cpu(clock, out)?,
                Ok(Exit::Reset) => {
                    self.board.sync_serial(out)?;
                    self.board.reset_mcu(&mut self.cpu, true, out)?;
                    self.queue_cpu(clock, out)?;
                }
                Err(error) => {
                    // A stopped session retains the serial effects preceding the fault.
                    self.board.sync_serial(out)?;
                    return Err(error);
                }
            }
        }
    }
}

/// A word access to a byte-wide owner is issued as two physical byte lanes.
fn split_word(access: Access) -> bool {
    access.width == Width::Word && !access.native_word()
}
fn pending_split(action: Action) -> bool {
    match action {
        Action::Read { address, width, .. } => split_word(Mcu::classify(address, width, false)),
        Action::Write { address, width, .. } => split_word(Mcu::classify(address, width, true)),
        Action::Idle(_) | Action::Sleep => false,
    }
}

impl Board {
    fn execution_boundary(&self, end: Time, input: Option<Time>) -> Result<Time, Error> {
        Ok(end
            .min(self.next_devices.unwrap_or(end))
            .min(input.unwrap_or(end))
            .min(self.resume_deadline()?.unwrap_or(end)))
    }
    /// Exception acceptance protects flash and acknowledges NMI at the time
    /// the CPU accepts it, before the entry sequence's first access.
    fn accept_exception(&mut self, vector: u8, out: &mut dyn Output) -> Result<(), Error> {
        self.mcu.flash.protect(self.now, out)?;
        if vector == 7 {
            self.mcu.control.acknowledge_nmi();
        }
        Ok(())
    }
    /// Commit one physical owner or flash transaction at `now`, synchronizing
    /// its owners first and settling the board's connections afterwards.
    /// `None` means a watchdog reset replaced the operation; the caller resets.
    fn transact(
        &mut self,
        access: Access,
        write: Option<(u16, WriteOrigin)>,
        _pc: u16,
        out: &mut dyn Output,
    ) -> Result<Option<Transacted>, Error> {
        let memory = access.memory();
        // SSU registers observe the serial engine's progress.
        let serial_changed = !memory
            && (cfg!(feature = "trace") || access.observes_serial() || access.serial_engine())
            && self.sync_serial(out)?;
        if !memory && self.mcu.sync_peripherals(access.owners, self.now, out)? {
            return Ok(None);
        }
        if access.owners & schedule::SCI != 0 {
            self.report_optical_before(Time::from_raw(self.now.raw() + 1), out)?;
        }
        // Whether the access, repeated before the next appointment, would
        // see and leave the board the same.
        let (value, effects, recurs) = match write {
            None => {
                self.stats.bus_reads = self.stats.bus_reads.wrapping_add(1);
                let value = self.mcu.read_access(access, self.now, out)?;
                let effects = access.read_changes_state().then_some(access.owners);
                let settled = self.settled(access);
                (value, effects, settled && effects.is_none())
            }
            Some((value, origin)) => {
                self.stats.bus_writes = self.stats.bus_writes.wrapping_add(1);
                let written = self
                    .mcu
                    .write_access(access, value, origin, self.now, out)?;
                (value, written.owners, written.recurs)
            }
        };
        #[cfg(feature = "trace")]
        let _ = out.event(Event::Bus {
            at: self.now,
            pc: _pc,
            address: access.address,
            width: access.width.bytes(),
            write: write.is_some(),
            value,
        });
        let changed = effects.filter(|_| !memory);
        if let Some(owners) = changed {
            self.settle_board(
                if access.configures_board() {
                    BoardChange::Configuration
                } else {
                    BoardChange::Peripherals {
                        owners,
                        clock_output: false,
                        serial_devices: false,
                    }
                },
                out,
            )?;
            if access.address == 0xffc0 {
                self.refresh_board_appointments(1 << 2)?;
            }
            self.refresh_peripherals(
                owners
                    | if access.configures_board() {
                        schedule::SSU
                    } else {
                        0
                    },
            )?;
        }
        let changed = changed.is_some() || serial_changed;
        Ok(Some(Transacted {
            value,
            changed,
            disturbs: serial_changed || !recurs,
        }))
    }
}

/// A committed owner or flash transaction.
struct Transacted {
    value: u16,
    /// The board's state or connections changed and were settled.
    changed: bool,
    /// The access could make a repetition of the work around it differ:
    /// it changed state, or observed state that can change, before the
    /// next appointment.
    disturbs: bool,
}

/// Register accesses inside direct work: completed by their owners alone
/// (`Registers::local`) at the time the window gives their completion, and
/// SSU accesses by the serial engine when it is lazy (`Wires::lazy`).
struct LocalRegisters<'a, 'w, 'o> {
    registers: Registers<'a>,
    devices: SerialDevices<'a>,
    out: &'w mut Delivery<'o>,
    window: &'w CpuWindow,
    /// The CPU clock position of the window's start.
    start: u64,
    /// An access may have changed the offered request, or output asked to stop.
    stopped: bool,
    error: Option<Error>,
    /// The serial engine, once an SSU access has looked: None when not lazy.
    engine: Option<Option<Engine>>,
    /// `Register::until` for the access being completed.
    until: Option<u64>,
    /// While the SCI is lazy, the first window edge at which an access can
    /// see its registers changed (`Sci::flag_deadline`); None until looked.
    sci_due: Option<Option<u64>>,
}
impl LocalRegisters<'_, '_, '_> {
    /// An SSU access at the `elapsed`-th edge, after the serial engine has
    /// performed every SSU event that can affect the access. Lazy SSU events are
    /// observable only here, at the serial bus and in the output, so the
    /// engine can perform them now.
    #[inline(never)]
    fn serial(
        &mut self,
        access: Access,
        write: Option<(u16, WriteOrigin)>,
        elapsed: u64,
    ) -> Result<Option<Local>, Error> {
        if write.is_some() && !matches!(access.address, 0xf0eb | 0xf0e4) {
            return Ok(None);
        }
        let window = self.window;
        let (ssu, gpio, iic, clocks) = self.registers.serial_parts();
        let mut wires = Wires {
            ssu,
            gpio,
            iic,
            clocks,
            devices: &mut self.devices,
            out: &mut *self.out,
            now: Time::ZERO,
        };
        // Nothing but declined accesses, which end the work, changes whether
        // the engine is lazy or its net (`Wires::run_on`).
        if self.engine.is_none() {
            self.engine = Some(match wires.lazy_net() {
                Some(net) => {
                    let (due, flags_due) = Self::due(wires.ssu, clocks, window, self.start)?;
                    Some(Engine {
                        net,
                        due,
                        flags_due,
                    })
                }
                None => None,
            });
        }
        let Some(engine) = self.engine.as_mut().expect("serial engine initialized") else {
            return self
                .registers
                .local(access, write, || window.time_of(elapsed));
        };
        // Live MOSI and RDR reads need intermediate work: RDR can rearm
        // reception after a frame stops. Holding data and flag writes next
        // interact with a load or sample/completion in the flag deadline.
        let due = if write.is_none() && Ssu::reads_shift(access.address) {
            engine.due
        } else {
            engine.flags_due
        };
        let ran = elapsed >= due;
        let at = if ran {
            let at = window.time_of(elapsed)?;
            let through = Some((window, self.start + elapsed));
            if wires
                .run_on(&engine.net, Time::from_raw(at.raw() + 1), through)?
                .is_some()
            {
                return Ok(None);
            }
            Some(at)
        } else {
            None
        };
        let at = || at.map_or_else(|| window.time_of(elapsed), Ok);
        let (a, ssu) = (access.address, &mut *wires.ssu);
        let rearmed = write.is_none() && ssu.read_rearms(a);
        let value = match write {
            None => u16::from(ssu.read_at(a, at, clocks)?),
            // Data and flag clearing move no pin and leave the engine lazy.
            Some((value, origin)) if matches!(a, 0xf0eb | 0xf0e4) => {
                ssu.write_at(
                    a,
                    value as u8,
                    origin.is_mov(),
                    || {
                        Ok(Instant {
                            time: at()?,
                            cpu: Some(self.start + elapsed),
                        })
                    },
                    clocks,
                )?;
                value
            }
            Some(_) => return Ok(None),
        };
        // A status read before the flags can change repeats the same way
        // until they can (`flags_due`).
        let settled = !ran && write.is_none() && a == 0xf0e4;
        if settled {
            self.until = Some(self.start.saturating_add(engine.flags_due - 1));
        }
        // Retained reads and duplex RDR consumption leave the schedule alone.
        if ran || write.is_some() || rearmed {
            (engine.due, engine.flags_due) = Self::due(ssu, clocks, window, self.start)?;
        }
        // A lazy engine has no enabled SSU interrupt; accepted writes do not
        // change its enables. Only output can request a wider settlement.
        let requests = self.out.stopped;
        Ok(Some(Local {
            value,
            disturbs: !settled,
            requests,
        }))
    }
    /// An access to the SCI's owners at the `elapsed`-th edge. A lazy SCI's
    /// registers change only by steps it performs when an access reaches
    /// them, so until then its accesses repeat alike.
    fn sci_local(
        &mut self,
        access: Access,
        write: Option<(u16, WriteOrigin)>,
        elapsed: u64,
    ) -> Result<Option<Local>, Error> {
        let window = self.window;
        let due = match self.sci_due {
            Some(due) => due,
            None => {
                let due = self
                    .registers
                    .sci_flag_deadline()?
                    .map(|at| at.map_or(u64::MAX, |at| window.edges_before(at) + 1));
                self.sci_due = Some(due);
                due
            }
        };
        let Some(due) = due else {
            return self
                .registers
                .local(access, write, || window.time_of(elapsed));
        };
        let reached = elapsed >= due;
        if reached {
            self.registers
                .sync(schedule::SCI, window.time_of(elapsed)?)?;
            self.sci_due = None;
        }
        let result = self
            .registers
            .local(access, write, || window.time_of(elapsed))?;
        Ok(result.map(|mut done| {
            done.disturbs |= reached;
            if done.disturbs {
                self.sci_due = None;
            } else {
                self.until = Some(self.start.saturating_add(due - 1));
            }
            done
        }))
    }
    /// First window edges reaching an SSU event and a flag consequence.
    fn due(
        ssu: &Ssu,
        clocks: &Clocks,
        window: &CpuWindow,
        start: u64,
    ) -> Result<(u64, u64), Error> {
        if let Some((deadline, flags)) = ssu.due_ordinals(clocks) {
            // An event on the CPU clock's edge `k` is due from window edge
            // `k - start`, and from the first if it is not after the start.
            let edge = |at: Option<u64>| at.map_or(u64::MAX, |k| k.saturating_sub(start).max(1));
            return Ok((edge(deadline), edge(flags)));
        }
        let edge = |at: Option<Time>| at.map_or(u64::MAX, |at| window.edges_before(at) + 1);
        Ok((
            edge(ssu.deadline(clocks)?),
            edge(ssu.flag_deadline(clocks)?),
        ))
    }
}
/// The lazy serial engine within direct work (`LocalRegisters::serial`).
struct Engine {
    net: ShiftNet,
    /// The first window edge an SSU event precedes or coincides with.
    due: u64,
    /// The same for events that can change a status flag.
    flags_due: u64,
}
impl RegisterBus for LocalRegisters<'_, '_, '_> {
    fn access(
        &mut self,
        address: u16,
        width: Width,
        write: Option<(u16, WriteOrigin)>,
        begin: u64,
    ) -> Option<Register> {
        if self.error.is_some() {
            return None;
        }
        if write.is_none() && width == Width::Byte && address == 0xf0e4 {
            const STATES: u64 = Mcu::access_states(0xf0e4, Width::Byte);
            if let Some(Some(engine)) = &self.engine {
                if begin + STATES < engine.flags_due {
                    let (ssu, _, _, _) = self.registers.serial_parts();
                    return Some(Register {
                        value: u16::from(ssu.read_status()),
                        states: STATES as u8,
                        disturbs: false,
                        stop: false,
                        until: self.start.saturating_add(engine.flags_due - 1),
                    });
                }
            }
        }
        let access = Mcu::classify(address, width, write.is_some());
        let states = access.states();
        if states > u64::from(REGISTER_STATES) {
            return None;
        }
        let window = self.window;
        let result = if access.serial_engine() && access.width == Width::Byte {
            self.serial(access, write, begin + states)
        } else if access.owners & schedule::SCI != 0 {
            self.sci_local(access, write, begin + states)
        } else {
            self.registers
                .local(access, write, || window.time_of(begin + states))
        };
        match result {
            Ok(Some(done)) => {
                self.stopped |= done.requests;
                Some(Register {
                    value: done.value,
                    states: states as u8,
                    disturbs: done.disturbs,
                    stop: done.requests,
                    until: self.until.take().unwrap_or(u64::MAX),
                })
            }
            Ok(None) => None,
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }
}

/// The board as the CPU's bus for one interval. RAM and normally readable
/// flash complete without owner involvement; everything else transacts.
struct Execution<'a, 'o> {
    board: &'a mut Board,
    out: &'a mut Delivery<'o>,
    clock: &'a mut CpuCursor,
    window: CpuWindow,
    revision: u64,
    boundary: Time,
    input: Option<Time>,
    /// The run's inputs and how many are applied.
    inputs: &'a [TimedInput],
    consumed: &'a mut usize,
    /// The interval starts as the pending access completes.
    first_due: bool,
    /// The pending access resumes at its second byte lane with this high byte.
    resume_lane: Option<u8>,
    /// A split word access stopped between its byte lanes, with the high byte.
    suspended_lane: Option<(Action, u8)>,
    overshoot: Option<u64>,
    /// The last device boundary served inside the interval.
    served: Option<Time>,
    fenced: bool,
    interrupt: Option<u8>,
    expired_interrupt: Option<u8>,
    normal_flash: Option<NormalRead>,
    /// The next instruction boundary has MCU state to expire.
    boundary_pending: bool,
    pc: u16,
    reads: u64,
    writes: u64,
    /// Accesses that changed state or observed state that can change before
    /// the boundary, and accepted exceptions (see `Mark`).
    disturbances: u64,
    /// `DirectRun::horizon`, as a CPU clock position.
    horizon: u64,
}
impl<'a, 'o> Execution<'a, 'o> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        board: &'a mut Board,
        out: &'a mut Delivery<'o>,
        clock: &'a mut CpuCursor,
        window: CpuWindow,
        boundary: Time,
        inputs: &'a [TimedInput],
        consumed: &'a mut usize,
        pending: Pending,
        at: Time,
        pc: u16,
    ) -> Result<Self, Error> {
        let revision = board.mcu.clocks.revision();
        let input = inputs.get(*consumed).map(|i| i.at);
        let mut bus = Self {
            board,
            out,
            clock,
            window,
            revision,
            boundary,
            input,
            inputs,
            consumed,
            first_due: true,
            resume_lane: (pending.lane == 1).then_some(pending.high),
            suspended_lane: None,
            overshoot: None,
            served: None,
            fenced: false,
            interrupt: None,
            expired_interrupt: None,
            normal_flash: None,
            boundary_pending: false,
            pc,
            reads: 0,
            writes: 0,
            disturbances: 0,
            horizon: u64::MAX,
        };
        bus.observe_board(at)?;
        Ok(bus)
    }
    /// Re-derive what this interval caches from board state at time `at`.
    fn observe_board(&mut self, at: Time) -> Result<(), Error> {
        let mcu = &self.board.mcu;
        self.interrupt = mcu.interrupt();
        self.expired_interrupt = mcu.expired_interrupt();
        self.normal_flash = mcu.flash.normal_read(at);
        self.boundary_pending = mcu.boundary_pending();
        Ok(())
    }
    /// After a transaction at `now`: the boundary may have moved, the CPU
    /// clock may have changed or stopped, and the offered request may differ.
    fn settle(&mut self, now: Time, changed: bool) -> Result<(), Error> {
        let clocks = &self.board.mcu.clocks;
        if !clocks.available(Tap::cpu()) {
            self.fenced = true;
            self.window.close();
        } else if clocks.revision() != self.revision || changed || self.out.end < self.boundary {
            let boundary = self.board.execution_boundary(self.out.end, self.input)?;
            let moved = boundary != self.boundary;
            self.boundary = boundary;
            if clocks.revision() != self.revision {
                self.revision = clocks.revision();
                self.window.finish(self.clock)?;
                self.window = self
                    .clock
                    .window(now, self.boundary, &self.board.mcu.clocks)?;
            } else if moved {
                self.window.bound(self.boundary);
            }
        }
        self.observe_board(now)
    }
    #[inline(always)]
    fn advance(&mut self, states: u64) -> Result<bool, Stop> {
        if self.first_due {
            self.first_due = false;
            return Ok(true);
        }
        if self.window.advance(states)? {
            return Ok(true);
        }
        self.stall(states)
    }
    /// An access that cannot complete in this window: either the CPU clock
    /// stopped, so it never starts, or it completes at or after the horizon.
    #[cold]
    fn stall(&mut self, states: u64) -> Result<bool, Stop> {
        if self.fenced {
            self.window.retract(states);
            return Err(Stop::Fence);
        }
        while self.serve_boundary(states)? {
            if self.window.advance(states)? {
                return Ok(true);
            }
        }
        self.overshoot = Some(states);
        Ok(false)
    }
    /// Serve the device appointments at the boundary an access of `states`
    /// would pass, as the event loop would between intervals, and continue
    /// the interval past them: the access completes at or after the
    /// boundary, so it follows every effect there either way. Returns
    /// whether the window now extends past the boundary. Boundaries that
    /// involve the CPU itself, an input or the caller, and appointments that
    /// change the CPU clock, end the interval instead.
    fn serve_boundary(&mut self, states: u64) -> Result<bool, Error> {
        let at = self.boundary;
        if cfg!(feature = "trace") || at >= self.out.end {
            return Ok(false);
        }
        let board = &mut *self.board;
        let devices = board.next_devices == Some(at);
        let first = *self.consumed;
        let batch = if self.input == Some(at) {
            let end = first
                + self.inputs[first..]
                    .iter()
                    .take_while(|i| i.at == at)
                    .count();
            Some(end)
        } else {
            None
        };
        if (!devices && batch.is_none()) || board.resume_deadline()? == Some(at) {
            return Ok(false);
        }
        let now = board.now;
        board.now = at;
        let light = batch.and_then(|end| board.light_without_cpu(&self.inputs[first..end]));
        if (devices && !board.serves_without_cpu()?) || (batch.is_some() && light.is_none()) {
            board.now = now;
            return Ok(false);
        }
        self.window.retract(states);
        self.served = Some(at);
        if devices {
            board.devices_at_boundary(None, self.out)?;
        }
        if let (Some(end), Some(light)) = (batch, light) {
            board.sync_serial(self.out)?;
            board.apply_light(None, light, self.out)?;
            *self.consumed = end;
            board.queue_light(self.inputs, self.consumed, self.out.end)?;
            self.input = self.inputs.get(*self.consumed).map(|i| i.at);
        }
        // What the work around the boundary observes may differ (`Mark`).
        self.disturbances += 1;
        let clocks = &self.board.mcu.clocks;
        if !clocks.available(Tap::cpu()) || clocks.revision() != self.revision {
            self.window.advance(states)?;
            return Ok(false);
        }
        self.settle(at, true)?;
        Ok(true)
    }
    /// One physical transaction of `states`, completing at its own edge.
    fn lane(
        &mut self,
        address: u16,
        width: Width,
        write: Option<(u16, WriteOrigin)>,
    ) -> Result<Option<u16>, Stop> {
        let access = Mcu::classify(address, width, write.is_some());
        if !self.advance(access.states())? {
            return Ok(None);
        }
        // A settled value is current without synchronizing its owner, and
        // reading it changes nothing the interval derived.
        if write.is_none()
            && !access.serial_engine()
            && self.board.settled(access)
            && !cfg!(feature = "trace")
        {
            self.board.stats.bus_reads = self.board.stats.bus_reads.wrapping_add(1);
            let value = self.board.mcu.read_access_at(
                access,
                || Err(Error::Internal("settled read projected time")),
                self.out,
            )?;
            return Ok(Some(value));
        }
        let now = self.window.at()?;
        self.board.now = now;
        self.board.last_effect = now;
        let Some(transacted) = self.board.transact(access, write, self.pc, self.out)? else {
            return Err(Stop::Reset);
        };
        if transacted.disturbs {
            self.disturbances += 1;
        }
        self.settle(now, transacted.changed)?;
        Ok(Some(transacted.value))
    }
    /// Owner registers and flash outside normal reads.
    #[inline(never)]
    fn transaction(&mut self, action: Action) -> Result<u16, Stop> {
        let (address, width, write) = match action {
            Action::Read { address, width, .. } => (address, width, None),
            Action::Write {
                address,
                width,
                value,
                origin,
            } => (address, width, Some((value, origin))),
            Action::Idle(_) | Action::Sleep => {
                return Err(Error::Internal("timed non-transaction").into())
            }
        };
        let base = if width == Width::Word {
            address & !1
        } else {
            address
        };
        if !split_word(Mcu::classify(address, width, write.is_some())) {
            return self.lane(base, width, write)?.ok_or(Stop::Horizon(action));
        }
        // A resumed interval opens as the suspended second lane completes.
        let high = match self.resume_lane.take() {
            Some(high) => high,
            None => {
                let lane = write.map(|(value, origin)| (value >> 8, origin));
                match self.lane(base, Width::Byte, lane)? {
                    Some(high) => high as u8,
                    None => return Err(Stop::Horizon(action)),
                }
            }
        };
        match self.lane(base.wrapping_add(1), Width::Byte, write) {
            Ok(Some(low)) => Ok(u16::from_be_bytes([high, low as u8])),
            Ok(None) => {
                self.suspended_lane = Some((action, high));
                Err(Stop::Horizon(action))
            }
            Err(Stop::Fence) => {
                self.suspended_lane = Some((action, high));
                Err(Stop::Fence)
            }
            Err(stop) => Err(stop),
        }
    }
    /// Close the interval at its last completed access. Returns the completed
    /// high byte when a split word access is left pending at its second lane.
    fn finish(self) -> Result<Option<(Action, u8)>, Error> {
        let at = match self.overshoot {
            Some(states) => self.window.before(states)?,
            None => self.window.at()?,
        };
        // After a served boundary the board is at least there.
        let at = self.served.map_or(at, |served| at.max(served));
        self.board.now = at;
        self.board.last_effect = at;
        self.board.stats.bus_reads = self.board.stats.bus_reads.wrapping_add(self.reads);
        self.board.stats.bus_writes = self.board.stats.bus_writes.wrapping_add(self.writes);
        self.window.finish(self.clock)?;
        Ok(self.suspended_lane)
    }
    #[cfg(feature = "trace")]
    fn trace(&mut self, address: u16, width: Width, write: bool, value: u16) {
        if let Ok(at) = self.window.at() {
            let _ = self.out.event(Event::Bus {
                at,
                pc: self.pc,
                address,
                width: width.bytes(),
                write,
                value,
            });
        }
    }
}
impl Bus for Execution<'_, '_> {
    #[inline(always)]
    fn read(&mut self, address: u16, width: Width, fetch: bool) -> Result<u16, Stop> {
        let a = if width == Width::Word {
            address & !1
        } else {
            address
        };
        let value = match (bus::memory_target(a), self.normal_flash) {
            (Some(Target::Ram), _) => {
                if !self.advance(u64::from(bus::MEMORY_STATES))? {
                    return Err(Stop::Horizon(Action::Read {
                        address,
                        width,
                        fetch,
                    }));
                }
                self.board.mcu.read_ram(a, width)
            }
            (Some(Target::Flash), Some(normal)) => {
                if !self.advance(u64::from(bus::MEMORY_STATES))? {
                    return Err(Stop::Horizon(Action::Read {
                        address,
                        width,
                        fetch,
                    }));
                }
                self.board.mcu.flash.read_normal(normal, a, width)
            }
            _ => {
                return self.transaction(Action::Read {
                    address,
                    width,
                    fetch,
                })
            }
        };
        self.reads = self.reads.wrapping_add(1);
        #[cfg(feature = "trace")]
        self.trace(a, width, false, value);
        Ok(value)
    }
    #[inline(always)]
    fn write(
        &mut self,
        address: u16,
        width: Width,
        value: u16,
        origin: WriteOrigin,
    ) -> Result<(), Stop> {
        let a = if width == Width::Word {
            address & !1
        } else {
            address
        };
        if bus::memory_target(a) != Some(Target::Ram) {
            return self
                .transaction(Action::Write {
                    address,
                    width,
                    value,
                    origin,
                })
                .map(|_| ());
        }
        if !self.advance(u64::from(bus::MEMORY_STATES))? {
            return Err(Stop::Horizon(Action::Write {
                address,
                width,
                value,
                origin,
            }));
        }
        if self.board.mcu.write_ram(a, width, value) {
            self.disturbances += 1;
        }
        self.writes = self.writes.wrapping_add(1);
        #[cfg(feature = "trace")]
        self.trace(a, width, true, value);
        Ok(())
    }
    #[inline(always)]
    fn idle(&mut self, states: u32) -> Result<(), Stop> {
        if !self.advance(u64::from(states))? {
            return Err(Stop::Horizon(Action::Idle(states)));
        }
        Ok(())
    }
    fn sample(&mut self) -> Result<Option<u8>, Stop> {
        Ok(self.interrupt)
    }
    fn mark(&self) -> Option<Mark> {
        if cfg!(feature = "trace")
            || self.fenced
            || self.first_due
            || self.boundary_pending
            || self.overshoot.is_some()
        {
            return None;
        }
        Some(Mark {
            position: self.window.position(),
            disturbances: self.disturbances,
            reads: self.reads.wrapping_add(self.board.stats.bus_reads),
            writes: self.writes.wrapping_add(self.board.stats.bus_writes),
        })
    }
    fn repeat(&mut self, since: &Mark) -> u64 {
        let Some(now) = self.mark() else {
            return 0;
        };
        let repetitions = repeat(&mut self.window, since, &now, self.horizon);
        let (reads, writes) = now.accesses_since(since, repetitions);
        self.reads = self.reads.wrapping_add(reads);
        self.writes = self.writes.wrapping_add(writes);
        repetitions
    }
}
/// Spend the edges of as many whole repetitions of the iteration from
/// `since` to `now` as fit the window and stable-access horizon. The shared
/// mark rule retains a final real iteration for writes whose last effect
/// must occur at its own time.
fn repeat(window: &mut CpuWindow, since: &Mark, now: &Mark, horizon: u64) -> u64 {
    let period = now.position.wrapping_sub(since.position);
    let remaining = window
        .remaining()
        .min(horizon.saturating_sub(window.position()));
    let repetitions = now.repetitions_since(since, remaining);
    if repetitions == 0 || !window.spend(repetitions * period) {
        return 0;
    }
    repetitions
}
impl IntervalBus for Execution<'_, '_> {
    #[inline(always)]
    fn direct(&mut self, work: impl FnOnce(&mut Memory<'_>, &mut DirectRun)) -> Result<(), Error> {
        if cfg!(feature = "trace") || self.fenced || self.first_due || self.boundary_pending {
            return Ok(());
        }
        let Some(normal) = self.normal_flash else {
            return Ok(());
        };
        let (bus_reads, bus_writes) = (self.board.stats.bus_reads, self.board.stats.bus_writes);
        let mut run = DirectRun {
            elapsed: self.window.elapsed(),
            available: self.window.available(),
            base: self.window.position() - self.window.elapsed(),
            interrupt: self.interrupt,
            disturbances: self.disturbances,
            reads: self.reads.wrapping_add(bus_reads),
            writes: self.writes.wrapping_add(bus_writes),
            horizon: self.horizon,
        };
        let board = &mut *self.board;
        let (ram, flash, registers) = board.mcu.memory_parts();
        let mut registers = LocalRegisters {
            registers,
            devices: SerialDevices {
                lcd: &mut board.lcd,
                eeprom: &mut board.eeprom,
                sensor: &board.sensor,
                power: &board.power,
                serial: &mut board.serial,
                shift: &mut board.shift,
                edges: &mut board.stats.peripheral_boundaries,
                last_effect: &mut board.last_effect,
                late: board.late_output.0,
            },
            out: &mut *self.out,
            window: &self.window,
            start: self.window.position() - self.window.elapsed(),
            stopped: false,
            error: None,
            engine: None,
            until: None,
            sci_due: None,
        };
        let mut memory = Memory {
            ram,
            generation: flash.generation(),
            flash: flash.normal_bytes(normal),
            registers: &mut registers,
        };
        work(&mut memory, &mut run);
        let (stopped, error) = (registers.stopped, registers.error);
        self.window.set_elapsed(run.elapsed);
        self.disturbances = run.disturbances;
        self.horizon = run.horizon;
        self.reads = run.reads.wrapping_sub(bus_reads);
        self.writes = run.writes.wrapping_sub(bus_writes);
        if let Some(error) = error {
            return Err(error);
        }
        if stopped {
            let at = self.window.at()?;
            self.observe_board(at)?;
        }
        Ok(())
    }
    fn proceed(&self) -> bool {
        !self.fenced
    }
    fn interrupt(&self) -> Option<u8> {
        self.interrupt
    }
    #[inline(always)]
    fn instruction_boundary(&mut self) {
        if self.boundary_pending {
            self.board.mcu.instruction_boundary();
            self.boundary_pending = self.board.mcu.boundary_pending();
        }
        self.interrupt = self.expired_interrupt;
    }
    fn accept(&mut self, vector: u8) -> Result<(), Stop> {
        self.disturbances += 1;
        let now = self.window.at()?;
        self.board.now = now;
        self.board.accept_exception(vector, self.out)?;
        self.settle(now, true)?;
        Ok(())
    }
    #[cfg(feature = "trace")]
    fn instruction(&mut self, pc: u16) {
        self.pc = pc;
    }
}
