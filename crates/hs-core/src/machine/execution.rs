//! Execute CPU actions within the next external boundary. Accesses and power
//! transitions that change the schedule rebuild the local cycle budget.
use super::*;
use crate::mcu::clocks::CpuBudget;
use core::ops::ControlFlow;

// Output can shorten the horizon; owners finish the current timestamp first.
struct Delivery<'a> {
    output: &'a mut dyn Output,
    end: Time,
}
impl Output for Delivery<'_> {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        let flow = self.output.event(event);
        if flow.is_break() {
            self.end = self.end.min(Time::from_raw(event.time().raw() + 1));
        }
        flow
    }
}

impl Machine {
    // Report schedule changes so the caller can rebuild its cycle budget.
    fn queue_cpu(&mut self, clock: &mut CpuBudget, out: &mut dyn Output) -> Result<bool, Error> {
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
        if let Some(boot) = &mut self.boot {
            let action = boot.next(self.now, &self.mcu)?;
            if boot.done() {
                self.boot = None;
                self.cpu = Cpu::new(0xfb80);
                self.mcu.instruction_boundary();
            } else {
                if let Some(action) = action {
                    self.queue_action(action, clock)?;
                }
                return Ok(changed);
            }
        }
        if let Some(resume) = self.resume_after {
            if self.resume_deadline()? != Some(self.now) {
                return Ok(changed);
            }
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
            let boundary = self.cpu.boundary();
            let action = self.cpu.next(|| self.mcu.interrupt())?;
            if boundary {
                self.mcu.instruction_boundary();
            }
            if let Some(vector) = self.cpu.take_accepted_vector() {
                self.mcu.flash.protect(self.now, out)?;
                if vector == 7 {
                    self.mcu.control.acknowledge_nmi();
                }
            }
            match action {
                Action::Sleep => {
                    if self.mcu.control.sleeping() {
                        return Ok(changed);
                    }
                    changed = true;
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
    fn queue_action(&mut self, action: Action, clock: &mut CpuBudget) -> Result<(), Error> {
        let (states, split) = match action {
            Action::Idle(states) => (u64::from(states), false),
            Action::Read { address, width, .. } | Action::Write { address, width, .. } => {
                let address = if width == Width::Word {
                    address & !1
                } else {
                    address
                };
                let split = width == Width::Word && !Mcu::native_word(address);
                (
                    Mcu::access_states(address, if split { Width::Byte } else { width }),
                    split,
                )
            }
            Action::Sleep => return Err(Error::Internal("scheduled SLEEP access")),
        };
        self.pending = Some(Pending {
            action,
            wait: clock.after(self.now, states, &self.mcu.clocks)?,
            split,
            lane: 0,
            high: 0,
        });
        Ok(())
    }
    fn complete_action(&mut self, value: u16) -> Result<(), Error> {
        if let Some(boot) = &mut self.boot {
            boot.complete(value, self.mcu.clocks.frequencies.main_hz)
        } else {
            self.cpu.complete(value)
        }
    }
    fn complete_cpu(&mut self, clock: &mut CpuBudget, out: &mut dyn Output) -> Result<bool, Error> {
        self.last_effect = self.now;
        let mut pending = self
            .pending
            .take()
            .ok_or(Error::Internal("CPU completion without pending access"))?;
        let (address, width, write) = match pending.action {
            Action::Idle(_) => {
                self.complete_action(0)?;
                return Ok(false);
            }
            Action::Read { address, width, .. } => (address, width, false),
            Action::Write { address, width, .. } => (address, width, true),
            Action::Sleep => return Err(Error::Internal("scheduled SLEEP access")),
        };
        let base = if width == Width::Word {
            address & !1
        } else {
            address
        };
        let a = base.wrapping_add(u16::from(pending.lane));
        let w = if pending.split { Width::Byte } else { width };
        let memory = Mcu::is_memory(a);
        let affected = Mcu::access_peripherals(a, write);
        if !memory && self.mcu.sync_peripherals(affected, self.now, out)? {
            self.reset_mcu(true, out)?;
            return Ok(true);
        }
        let value = if let Action::Write {
            value, mov_byte, ..
        } = pending.action
        {
            let v = if pending.split && pending.lane == 0 {
                value >> 8
            } else {
                value
            };
            match w {
                Width::Byte => self.mcu.write8(
                    a,
                    v as u8,
                    if self.boot.is_some() {
                        WriteOrigin::MovByte
                    } else {
                        self.cpu.write_origin(mov_byte)
                    },
                    self.now,
                    out,
                )?,
                Width::Word => self.mcu.write16(a, v, self.now, out)?,
            };
            self.stats.bus_writes = self.stats.bus_writes.wrapping_add(1);
            v
        } else {
            self.stats.bus_reads = self.stats.bus_reads.wrapping_add(1);
            match w {
                Width::Byte => u16::from(self.mcu.read8(a, self.now, out)?),
                Width::Word => self.mcu.read16(a, self.now, out)?,
            }
        };
        #[cfg(feature = "trace")]
        let _ = out.event(Event::Bus {
            at: self.now,
            pc: self.cpu.instruction_pc(),
            address: a,
            width: w.bytes(),
            write,
            value,
        });
        // Due peripheral effects were settled before this access. Other reads
        // only observe state or qualify flags; they preserve pins and deadlines.
        let changed = !memory && (write || Mcu::read_starts_transfer(a));
        if changed {
            self.resolve_board(out)?;
            self.refresh_peripherals(affected)?;
        }
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
            self.complete_action(value)?;
        }
        Ok(changed)
    }
    /// Advance through effects before `end`, returning earlier if output requests it.
    /// Inputs at the returned exclusive horizon remain pending.
    pub fn run_until(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
    ) -> Result<RunResult, Error> {
        self.check_fault()?;
        self.validate_inputs(end, inputs)?;
        let result = self.run_inner(end, inputs, out);
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
    ) -> Result<RunResult, Error> {
        let mut out = Delivery { output: out, end };
        let mut clock = CpuBudget::new(&self.mcu.clocks);
        let mut consumed = 0;
        while self.now < out.end {
            // Settle simultaneous device and input causes before the CPU access.
            if self.next_devices == Some(self.now) {
                self.devices_at_boundary(&mut out)?;
            }
            if consumed < inputs.len() && inputs[consumed].at == self.now {
                let start = consumed;
                while consumed < inputs.len() && inputs[consumed].at == self.now {
                    consumed += 1;
                }
                self.apply_batch(&inputs[start..consumed], &mut out)?;
            }
            let input = inputs.get(consumed).map(|i| i.at);
            let mut boundary = self.execution_boundary(out.end, input)?;
            clock.bound(boundary, &self.mcu.clocks);
            let mut cpu_due = self.pending_deadline()? == Some(self.now);
            loop {
                let mut changed = false;
                if cpu_due {
                    changed = self.complete_cpu(&mut clock, &mut out)?;
                }
                changed |= self.queue_cpu(&mut clock, &mut out)?;
                if changed || out.end < boundary {
                    boundary = self.execution_boundary(out.end, input)?;
                    clock.bound(boundary, &self.mcu.clocks);
                }
                let next = self.pending.as_ref().map_or(Ok(None), |pending| {
                    clock.before_boundary(&pending.wait, &self.mcu.clocks)
                })?;
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
        Ok(RunResult {
            now: self.now,
            inputs_consumed: consumed,
            retired: self.cpu.retired,
        })
    }
}
