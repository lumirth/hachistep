//! Sensor power, acquisition readiness, autonomous verification and commands.
use super::*;

/// The stated readiness interval includes the first complete T/X/Y/Z scan.
pub(super) fn acquisition_delay(ready: Duration) -> Duration {
    Duration::from_raw(ready.raw().saturating_sub((1u128 << 64) / 3000))
}

impl Bma150 {
    pub(super) fn automatic(&self) -> bool {
        self.registers[0x15] & 1 != 0
    }
    pub(super) fn acquisition_cycles(&self) -> usize {
        // Bosch Rev.1.6 §7.3 p.47 omits filter fill at 1500 Hz. The first
        // complete scan is already included in the 1 ms wake-up interval.
        if self.window() == 1 {
            1
        } else {
            2 * self.window()
        }
    }
    pub(super) fn start_acquisition(&mut self, now: Time, ready: Duration) -> Result<(), Error> {
        for axis in &mut self.analog {
            axis.resume(now);
        }
        self.asleep = false;
        self.pause_deadline = None;
        self.wake_deadline = Some(
            now.checked_add(acquisition_delay(ready))
                .ok_or(crate::time::TimeError::Overflow)?,
        );
        self.filter = filter::Filter::new(self.registers[0x14] & 7);
        self.interrupts.restart_acquisition();
        self.shadows = [None; 3];
        self.auto_cycles = 0;
        Ok(())
    }
    fn pause(&mut self, now: Time) -> Result<(), Error> {
        if !self.asleep {
            self.hold_analog(now)?;
        }
        self.asleep = true;
        self.wake_deadline = None;
        self.pause_deadline = if self.automatic() {
            let ms = [20, 80, 320, 2560][usize::from(self.registers[0x15] >> 1 & 3)];
            Some(
                now.checked_add(Duration::from_millis(ms))
                    .ok_or(crate::time::TimeError::Overflow)?,
            )
        } else {
            None
        };
        self.driven = Drive::Floating;
        Ok(())
    }
    pub(super) fn automatic_control(
        &mut self,
        now: Time,
        was_criterion: bool,
        verify: bool,
    ) -> Result<(), Error> {
        if !self.automatic() {
            self.irq_hold = None;
            self.pause_deadline = None;
            return Ok(());
        }
        if self.asleep && self.pause_deadline.is_none() {
            self.pause(now)?;
        }
        // New-data is independently read-acknowledged. Only nonlatched
        // criterion interrupts receive the automatic mode's minimum width.
        if !was_criterion
            && self.registers[0x15] & 0x10 == 0
            && self.interrupts.output(&self.registers)
        {
            self.irq_hold = Some(
                now.checked_add(Duration::from_micros(330))
                    .ok_or(crate::time::TimeError::Overflow)?,
            );
        }
        if verify
            && !self.asleep
            && self.wake_deadline.is_none()
            && self.test_phases.is_none()
            && self.image_deadline.is_none()
            && self.nv_operation.is_none()
            && usize::from(self.auto_cycles) >= self.acquisition_cycles()
            && !self.interrupt()
            && !self.interrupts.verifying(&self.registers)
        {
            self.pause(now)?;
        }
        Ok(())
    }
    fn soft_reset(&mut self, now: Time) -> Result<(), Error> {
        self.i2c = i2c::State::default();
        let ready = if self.asleep {
            Duration::from_millis(30)
        } else {
            Duration::from_micros(1300)
        };
        self.analog = [analog::Axis::new(now, 0); 3];
        self.registers.fill(0);
        self.registers[0] = 2;
        self.registers[1] = 0x10;
        self.interrupts = interrupts::Interrupts::default();
        self.copy_image();
        self.filtered = [0; 3];
        self.last_msb = [0; 3];
        self.data_ready = false;
        self.irq_hold = None;
        self.test_phases = None;
        self.image_deadline = None;
        self.tx = None;
        self.driven = Drive::Floating;
        self.quiet_deadline = Some(
            now.checked_add(Duration::from_micros(10))
                .ok_or(crate::time::TimeError::Overflow)?,
        );
        self.start_acquisition(now, ready)
    }
    pub(super) fn write_register(
        &mut self,
        address: u8,
        value: u8,
        now: Time,
    ) -> Result<(), Error> {
        let i = usize::from(address);
        if i >= COUNT
            || address <= 9
            || self.quiet_deadline.is_some()
            || (self.image_deadline.is_some()
                && ((0x0b..=0x1d).contains(&address) || address >= 0x2b))
        {
            return Ok(());
        }
        if self.sleeping() && (address != 0x0a || value & 3 == 1) {
            return Ok(());
        }
        if address >= 0x16 && self.registers[0x0a] & 0x10 == 0 {
            return Ok(());
        }
        if address >= 0x2b {
            if self.nv_operation.is_none() {
                self.nv_operation = Some((
                    WriteCycle::start(now, Duration::from_millis(28))?,
                    address,
                    value,
                ));
            }
            return Ok(());
        }
        let was_criterion = self.interrupts.output(&self.registers);
        match address {
            0x0a if value & 2 != 0 => self.soft_reset(now)?,
            0x0a => {
                let old = self.registers[i];
                self.registers[i] = value & 0x3d;
                if value & 0x40 != 0 {
                    self.data_ready = false;
                    self.irq_hold = None;
                    self.interrupts.reset(&self.registers);
                    if self.automatic() {
                        self.auto_cycles = 0;
                        self.interrupts.restart_acquisition();
                    }
                }
                if value & 4 != 0 && old & 4 == 0 {
                    self.test_phases = Some(4);
                    self.registers[9] &= !0x80;
                } else if value & 4 == 0 {
                    self.test_phases = None;
                }
                if self.asleep && value & 1 == 0 {
                    self.start_acquisition(now, Duration::from_millis(1))?;
                } else if !self.asleep && value & 1 != 0 {
                    self.pause(now)?;
                }
                if value & 0x20 != 0 {
                    self.image_deadline = Some(
                        now.checked_add(Duration::from_micros(300))
                            .ok_or(crate::time::TimeError::Overflow)?,
                    );
                }
            }
            0x14 => {
                self.registers[i] = value;
                self.filter.select(value & 7);
            }
            0x15 => {
                let old = self.registers[i];
                self.registers[i] = value;
                if value & 8 != 0 {
                    self.shadows = [None; 3];
                }
                if (old ^ value) & 1 != 0 {
                    self.auto_cycles = 0;
                    self.interrupts.restart_acquisition();
                    self.irq_hold = None;
                }
            }
            0x0d | 0x0f => {
                self.registers[i] = value;
                self.interrupts.configure(&self.registers);
            }
            _ => self.registers[i] = value,
        }
        self.automatic_control(now, was_criterion, false)
    }
}
