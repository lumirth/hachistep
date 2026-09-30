//! Execute CPU actions before the next device, input, or caller boundary.
//! Reuse exact action deadlines while the clock configuration remains stable.
use super::*;
use crate::cpu::Request;
use crate::mcu::bus::Access;
use crate::mcu::clocks::CpuCursor;
use core::ops::ControlFlow;

// Output can shorten the horizon; owners finish the current timestamp first.
struct Delivery<'a> {
    output: &'a mut dyn Output,
    #[cfg(feature = "trace")]
    trace: Option<&'a mut dyn crate::trace::BusTrace>,
    end: Time,
    stopped: bool,
}
impl Delivery<'_> {
    fn tracing(&self) -> bool {
        #[cfg(feature = "trace")]
        return self.trace.is_some();
        #[cfg(not(feature = "trace"))]
        false
    }
    fn stop_at(&mut self, at: Time) {
        self.stopped = true;
        self.end = self.end.min(Time::from_raw(at.raw() + 1));
    }
    #[cfg(feature = "trace")]
    fn bus(&mut self, event: crate::trace::BusEvent) {
        if let Some(trace) = &mut self.trace {
            if trace.event(event).is_break() {
                self.stop_at(event.at);
            }
        }
    }
}
impl Output for Delivery<'_> {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        let flow = self.output.event(event);
        if flow.is_break() {
            self.stop_at(event.time());
        }
        flow
    }
}

impl Machine {
    // Report schedule changes so the caller can recompute the next boundary.
    fn queue_cpu(&mut self, clock: &mut CpuCursor, out: &mut dyn Output) -> Result<bool, Error> {
        if self.pending.is_some()
            || self.reset_asserted
            || self.reset_release.is_some()
            || self.watchdog_reset.is_some()
            || !self.power.mcu()
            || (!self.mcu.clocks.available(Tap::cpu()) && !self.cpu.sleeping())
        {
            return Ok(false);
        }
        let mut changed = false;
        if let Some(resume) = self.resume_after {
            if self.resume_deadline()? != Some(self.now) {
                return Ok(changed);
            }
            self.sync_serial(out)?;
            self.resume_after = None;
            changed = true;
            match resume {
                Resume::Sleep(_) => self.enter_sleep(out)?,
                Resume::Wake { direct, .. } => {
                    if self.mcu.sync(self.now, out)? {
                        self.reset_mcu(true, out)?;
                    } else {
                        self.mcu.control.stabilizing_from = None;
                        self.mcu.apply_gates(self.now, out)?;
                        self.resolve_board(out)?;
                        self.refresh_deadline()?;
                        if direct {
                            self.cpu.direct_transition()?;
                        }
                    }
                }
            }
            if self.resume_after.is_some() {
                return Ok(changed);
            }
        }
        if self.cpu.sleeping() && self.mcu.control.sleeping() {
            let irq = self.mcu.interrupt();
            if irq.is_none() || (irq != Some(7) && self.cpu.registers.ccr & I != 0) {
                return Ok(changed);
            }
            if self.mcu.control.sleeping() {
                changed = true;
                self.sync_serial(out)?;
                if self.mcu.sync(self.now, out)? {
                    self.reset_mcu(true, out)?;
                    return Ok(changed);
                }
                let wait = self.mcu.control.wake(self.now, &mut self.mcu.clocks)?;
                self.mcu.apply_gates(self.now, out)?;
                self.resolve_board(out)?;
                self.refresh_deadline()?;
                if let Some(wait) = wait {
                    self.resume_after = Some(Resume::Wake {
                        wait,
                        direct: false,
                    });
                    return Ok(changed);
                }
            }
        }
        loop {
            let (action, _) = match self.next_cpu_action(self.mcu.interrupt(), out) {
                Ok(next) => next,
                Err(error) => {
                    self.sync_serial(out)?;
                    return Err(error);
                }
            };
            match action {
                Action::Sleep => {
                    if self.mcu.control.sleeping() {
                        return Ok(changed);
                    }
                    changed = true;
                    self.sync_serial(out)?;
                    self.mcu.flash.protect(self.now, out)?;
                    if self.mcu.sync(self.now, out)? {
                        self.reset_mcu(true, out)?;
                        continue;
                    }
                    if self.mcu.control.sys2 & 8 != 0 {
                        self.resume_after = Some(Resume::Sleep(ClockWait::after(
                            self.now,
                            1,
                            Tap::cpu(),
                            &self.mcu.clocks,
                        )?));
                    } else {
                        self.enter_sleep(out)?;
                    }
                    return Ok(changed);
                }
                action => {
                    self.queue_action(action, clock)?;
                    return Ok(changed);
                }
            }
        }
    }
    fn next_cpu_action(
        &mut self,
        interrupt: Option<u8>,
        out: &mut dyn Output,
    ) -> Result<(Action, bool), Error> {
        let request = self.cpu.request(interrupt)?;
        self.apply_cpu_request(request, out)
    }
    fn apply_cpu_request(
        &mut self,
        request: Request,
        out: &mut dyn Output,
    ) -> Result<(Action, bool), Error> {
        let mut changed = request.admission && self.mcu.instruction_boundary();
        if let Some(vector) = request.exception {
            self.mcu.flash.protect(self.now, out)?;
            if vector == 7 {
                self.mcu.control.acknowledge_nmi();
                changed = true;
            }
        }
        Ok((request.action, changed))
    }
    fn queue_action(&mut self, action: Action, clock: &mut CpuCursor) -> Result<(), Error> {
        let (states, split) = Self::action_timing(action, Self::classify_action(action))?;
        self.pending = Some(Pending {
            action,
            wait: clock.after(self.now, states, &self.mcu.clocks)?,
            split,
            lane: 0,
            high: 0,
        });
        Ok(())
    }
    fn classify_action(action: Action) -> Option<Access> {
        match action {
            Action::Read { address, width, .. } => Some(Mcu::classify(address, width, false)),
            Action::Write { address, width, .. } => Some(Mcu::classify(address, width, true)),
            _ => None,
        }
    }
    fn action_timing(action: Action, access: Option<Access>) -> Result<(u64, bool), Error> {
        Ok(match action {
            Action::Idle(states) => (u64::from(states), false),
            Action::Read { .. } | Action::Write { .. } => {
                let access = access.ok_or(Error::Internal("unclassified CPU access"))?;
                (
                    access.states(),
                    access.width == Width::Word && !access.native_word(),
                )
            }
            Action::Sleep => return Err(Error::Internal("scheduled SLEEP access")),
        })
    }
    fn complete_action(&mut self, value: u16, out: &mut dyn Output) -> Result<(), Error> {
        let result = self.cpu.complete(value);
        // A stopped session must retain the serial effects preceding the fault.
        if result.is_err() {
            self.sync_serial(out)?;
        }
        result
    }
    fn complete_cpu(
        &mut self,
        clock: &mut CpuCursor,
        out: &mut Delivery<'_>,
    ) -> Result<bool, Error> {
        self.last_effect = self.now;
        let mut pending = self
            .pending
            .take()
            .ok_or(Error::Internal("CPU completion without pending access"))?;
        let (address, width) = match pending.action {
            Action::Idle(_) => {
                self.complete_action(0, out)?;
                return Ok(false);
            }
            Action::Read { address, width, .. } | Action::Write { address, width, .. } => {
                (address, width)
            }
            Action::Sleep => return Err(Error::Internal("scheduled SLEEP access")),
        };
        let base = if width == Width::Word {
            address & !1
        } else {
            address
        };
        let a = base.wrapping_add(u16::from(pending.lane));
        let w = if pending.split { Width::Byte } else { width };
        let action = if let Action::Write {
            value, mov_byte, ..
        } = pending.action
        {
            Action::Write {
                address: a,
                width: w,
                mov_byte,
                value: if pending.split && pending.lane == 0 {
                    value >> 8
                } else {
                    value
                },
            }
        } else {
            Action::Read {
                address: a,
                width: w,
                fetch: false,
            }
        };
        let Some((value, changed)) =
            self.commit_access(action, Self::classify_action(action), out)?
        else {
            return Ok(true);
        };
        if pending.split && pending.lane == 0 {
            pending.high = value as u8;
            pending.lane = 1;
            pending.wait = clock.after(
                self.now,
                Mcu::access_states(a.wrapping_add(1), Width::Byte),
                &self.mcu.clocks,
            )?;
            self.pending = Some(pending);
        } else {
            let value = if pending.split {
                u16::from_be_bytes([pending.high, value as u8])
            } else {
                value
            };
            self.complete_action(value, out)?;
        }
        Ok(changed)
    }
    // Commit one physical access, synchronizing its owners and connections.
    // None means that a watchdog reset replaced the CPU operation.
    fn commit_access(
        &mut self,
        action: Action,
        access: Option<Access>,
        out: &mut Delivery<'_>,
    ) -> Result<Option<(u16, bool)>, Error> {
        match action {
            Action::Idle(_) => return Ok(Some((0, false))),
            Action::Read { .. } | Action::Write { .. } => {}
            Action::Sleep => return Err(Error::Internal("SLEEP physical access")),
        };
        let access = access.ok_or(Error::Internal("unclassified CPU access"))?;
        let Some(serial_changed) = self.before_access(access, out)? else {
            return Ok(None);
        };
        let (value, effects) = self.access_cpu(action, access, out)?;
        let changed = self.after_access(access, effects, out)?;
        Ok(Some((value, changed || serial_changed)))
    }
    // Settle the old interval before a physical owner transaction mutates it.
    fn before_access(
        &mut self,
        access: Access,
        out: &mut Delivery<'_>,
    ) -> Result<Option<bool>, Error> {
        let memory = access.memory();
        let serial_changed =
            !memory && (out.tracing() || access.observes_serial()) && self.sync_serial(out)?;
        if !memory && self.mcu.sync_peripherals(access.owners, self.now, out)? {
            self.reset_mcu(true, out)?;
            return Ok(None);
        }
        Ok(Some(serial_changed))
    }
    // Both local committed replies and ordinary owner delivery apply this same
    // connection/appointment settlement before CPU semantic completion.
    fn after_access(
        &mut self,
        access: Access,
        effects: bool,
        out: &mut dyn Output,
    ) -> Result<bool, Error> {
        let changed = !access.memory() && effects;
        if changed {
            self.settle_board(
                if access.configures_board() {
                    BoardChange::Configuration
                } else {
                    BoardChange::Peripherals {
                        owners: access.owners,
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
                access.owners
                    | if access.configures_board() {
                        schedule::SSU
                    } else {
                        0
                    },
            )?;
        }
        Ok(changed)
    }
    // Both local execution and suspended accesses commit through this bus path.
    fn access_cpu(
        &mut self,
        action: Action,
        access: Access,
        out: &mut Delivery<'_>,
    ) -> Result<(u16, bool), Error> {
        let (address, width, write, value, effects) = match action {
            Action::Idle(_) => return Ok((0, false)),
            Action::Read { .. } => {
                self.stats.bus_reads = self.stats.bus_reads.wrapping_add(1);
                let value = self.mcu.read_access(access, self.now, out)?;
                (
                    access.address,
                    access.width,
                    false,
                    value,
                    access.read_changes_state(),
                )
            }
            Action::Write {
                value, mov_byte, ..
            } => {
                let effects = self.mcu.write_access(
                    access,
                    value,
                    self.cpu.write_origin(mov_byte),
                    self.now,
                    out,
                )?;
                self.stats.bus_writes = self.stats.bus_writes.wrapping_add(1);
                (access.address, access.width, true, value, effects)
            }
            Action::Sleep => return Err(Error::Internal("SLEEP bus access")),
        };
        #[cfg(feature = "trace")]
        out.bus(crate::trace::BusEvent {
            at: self.now,
            pc: self.cpu.instruction_pc(),
            address,
            width: width.bytes(),
            write,
            value: if width == Width::Byte {
                value & 0xff
            } else {
                value
            },
        });
        #[cfg(not(feature = "trace"))]
        let _ = (address, width, write);
        Ok((value, effects))
    }
    // Run the canonical CPU phase executor between interacting appointments.
    // An owner access commits one physical operation, then execution resumes
    // here with refreshed admission and timing state.
    fn advance_cpu(
        &mut self,
        boundary: Time,
        clock: &mut CpuCursor,
        out: &mut Delivery<'_>,
    ) -> Result<bool, Error> {
        loop {
            let Some(pending) = self.pending else {
                return Ok(false);
            };
            if pending.split {
                return Ok(false);
            }
            let Some(at) = pending.wait.deadline(&self.mcu.clocks)? else {
                return Ok(false);
            };
            if at >= boundary {
                return Ok(false);
            }
            let window = clock.window(at, boundary, &self.mcu.clocks)?;
            let mut bus = self.mcu.interval(window, at)?;
            #[cfg(feature = "profile-work")]
            self.work.interval_entries.add(1);
            if out.tracing() || !bus.can_serve(pending.action) {
                #[cfg(feature = "profile-work")]
                self.work.request.add(1);
                self.now = at;
                let mut changed = self.complete_cpu(clock, out)?;
                changed |= self.queue_cpu(clock, out)?;
                if changed || out.end < boundary {
                    return Ok(changed);
                }
                continue;
            }
            self.pending = None;
            let result = self.cpu.run_interval(&mut bus);
            let committed = bus.take_commit();
            #[cfg(feature = "profile-work")]
            let (at, reads, writes) = bus.finish(clock).inspect_err(|_| {
                self.work.error.add(1);
            })?;
            #[cfg(not(feature = "profile-work"))]
            let (at, reads, writes) = bus.finish(clock)?;
            self.now = at;
            self.last_effect = at;
            self.stats.bus_reads = self.stats.bus_reads.wrapping_add(reads);
            self.stats.bus_writes = self.stats.bus_writes.wrapping_add(writes);
            match result {
                Ok(crate::cpu::execution::Exit::Horizon(action)) => {
                    #[cfg(feature = "profile-work")]
                    self.work.horizon.add(1);
                    self.pending = Some(Pending {
                        action,
                        wait: clock.wait(),
                        split: false,
                        lane: 0,
                        high: 0,
                    });
                    return Ok(false);
                }
                Ok(crate::cpu::execution::Exit::Request(action)) => {
                    #[cfg(feature = "profile-work")]
                    self.work.request.add(1);
                    self.queue_action(action, clock)?;
                }
                Ok(crate::cpu::execution::Exit::Exception(request)) => {
                    #[cfg(feature = "profile-work")]
                    self.work.exception.add(1);
                    let (action, _) = self.apply_cpu_request(request, out)?;
                    self.queue_action(action, clock)?;
                    return Ok(true);
                }
                Ok(crate::cpu::execution::Exit::Sleep) => {
                    #[cfg(feature = "profile-work")]
                    self.work.sleep.add(1);
                    return self.queue_cpu(clock, out);
                }
                Ok(crate::cpu::execution::Exit::CommittedOwner) => {
                    #[cfg(feature = "profile-work")]
                    self.work.committed_owner.add(1);
                    let committed =
                        committed.ok_or(Error::Internal("missing committed owner reply"))?;
                    self.after_access(committed.access, true, out)?;
                    self.complete_action(committed.value, out)?;
                    self.queue_cpu(clock, out)?;
                    return Ok(true);
                }
                Ok(crate::cpu::execution::Exit::Reset) => {
                    #[cfg(feature = "profile-work")]
                    self.work.reset.add(1);
                    self.sync_serial(out)?;
                    self.reset_mcu(true, out)?;
                    self.queue_cpu(clock, out)?;
                    return Ok(true);
                }
                Err(error) => {
                    #[cfg(feature = "profile-work")]
                    self.work.error.add(1);
                    self.sync_serial(out)?;
                    return Err(error);
                }
            }
        }
    }
    /// Advance through effects before `end`, returning earlier if output requests it.
    /// Inputs at the returned exclusive horizon remain pending.
    pub fn run_until(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
    ) -> Result<RunResult, Error> {
        self.run_observed(
            end,
            inputs,
            out,
            #[cfg(feature = "trace")]
            None,
        )
    }
    pub(crate) fn run_observed(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
        #[cfg(feature = "trace")] trace: Option<&mut dyn crate::trace::BusTrace>,
    ) -> Result<RunResult, Error> {
        self.check_fault()?;
        self.validate_inputs(end, inputs)?;
        let result = self.run_inner(
            end,
            inputs,
            out,
            #[cfg(feature = "trace")]
            trace,
        );
        self.latch_error(result)
    }
    fn execution_boundary(&self, end: Time, input: Option<Time>) -> Result<Time, Error> {
        Ok(end
            .min(self.next_devices.unwrap_or(end))
            .min(input.unwrap_or(end))
            .min(self.resume_deadline()?.unwrap_or(end)))
    }
    fn run_inner(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
        #[cfg(feature = "trace")] trace: Option<&mut dyn crate::trace::BusTrace>,
    ) -> Result<RunResult, Error> {
        let mut out = Delivery {
            output: out,
            #[cfg(feature = "trace")]
            trace,
            end,
            stopped: false,
        };
        let mut clock = CpuCursor::new(&self.mcu.clocks);
        let mut consumed = 0;
        while self.now < out.end {
            // Settle simultaneous device and input causes before the CPU access.
            if self.next_devices == Some(self.now) {
                self.devices_at_boundary(&mut out)?;
            }
            if consumed < inputs.len() && inputs[consumed].at == self.now {
                self.sync_serial(&mut out)?;
                let start = consumed;
                while consumed < inputs.len() && inputs[consumed].at == self.now {
                    consumed += 1;
                }
                self.apply_batch(&inputs[start..consumed], &mut out)?;
            }
            let input = inputs.get(consumed).map(|i| i.at);
            let mut boundary = self.execution_boundary(out.end, input)?;
            let mut cpu_due = self.pending_deadline()? == Some(self.now);
            loop {
                let mut changed = false;
                if cpu_due {
                    changed = self.complete_cpu(&mut clock, &mut out)?;
                }
                changed |= self.queue_cpu(&mut clock, &mut out)?;
                if !changed {
                    changed |= self.advance_cpu(boundary, &mut clock, &mut out)?;
                }
                if changed || out.end < boundary {
                    boundary = self.execution_boundary(out.end, input)?;
                }
                let next = self.pending_deadline()?.filter(|at| *at < boundary);
                let at = next.unwrap_or(boundary);
                if at <= self.now {
                    return Err(Error::Internal("non-advancing event loop"));
                }
                self.now = at;
                if next.is_none() {
                    break;
                }
                cpu_due = true;
            }
        }
        self.sync_serial_before(self.now, &mut out)?;
        self.sensor.sync_local_until(self.now, false)?;
        Ok(RunResult {
            now: self.now,
            inputs_consumed: consumed,
            reason: if out.stopped {
                StopReason::Output
            } else {
                StopReason::Horizon
            },
            retired: self.cpu.retired,
        })
    }
}
