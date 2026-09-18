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
    pub(super) fn start_acquisition(&mut self, now: Time, ready: Duration) -> Result<(), Error> {
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
            && usize::from(self.auto_cycles) >= 2 * self.window()
            && !self.interrupt()
            && !self.interrupts.verifying(&self.registers)
        {
            self.pause(now)?;
        }
        Ok(())
    }
    fn soft_reset(&mut self, now: Time) -> Result<(), Error> {
        let ready = if self.asleep {
            Duration::from_millis(30)
        } else {
            Duration::from_micros(1300)
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    fn t(us: u64) -> Time {
        Time::from_micros(us)
    }
    fn run(b: &mut Bma150, until: Time) {
        while let Some(at) = b.deadline().filter(|at| *at <= until) {
            b.at_deadline(at, &mut ()).unwrap();
        }
    }
    fn write(b: &mut Bma150, us: u64, address: u8, value: u8) {
        run(b, t(us));
        b.write_register(address, value, t(us)).unwrap();
    }
    #[test]
    fn first_complete_vectors_obey_cold_wake_and_reset_readiness() {
        let mut b = Bma150::new(Time::ZERO);
        run(&mut b, t(2999));
        assert_eq!(b.peek(7), Some(0));
        run(&mut b, t(3000));
        assert_eq!(b.peek(7), Some(32));
        write(&mut b, 3000, 0x0a, 1);
        b.set_input(Acceleration {
            x: 0,
            y: 0,
            z: -1_000_000,
        })
        .unwrap();
        write(&mut b, 3100, 0x14, 6); // Sleeping writes cannot change the range.
        assert_eq!(b.peek(0), None);
        write(&mut b, 4000, 0x0a, 0);
        run(&mut b, t(4999));
        assert_eq!(b.peek(7), Some(32));
        run(&mut b, t(5000));
        assert_eq!(b.peek(7), Some(224));
        write(&mut b, 6000, 0x0a, 2);
        assert_eq!(b.peek(0), None);
        write(&mut b, 6005, 0x12, 0x55);
        run(&mut b, t(6009));
        assert_eq!(b.peek(0), None);
        run(&mut b, t(6010));
        assert_eq!(b.peek(0), Some(2));
        assert_eq!(b.peek(0x12), Some(162));
        run(&mut b, t(7299));
        assert_eq!(b.peek(7), Some(0));
        run(&mut b, t(7300));
        assert_eq!(b.peek(7), Some(224));
        write(&mut b, 7500, 0x0a, 1);
        write(&mut b, 7600, 0x0a, 2);
        run(&mut b, t(37599));
        assert_eq!(b.peek(7), Some(0));
        run(&mut b, t(37600));
        assert_eq!(b.peek(7), Some(224));
        assert!(!b.sleeping());
    }
    #[test]
    fn image_update_blocks_image_access_until_completion() {
        let mut b = Bma150::new(Time::ZERO);
        write(&mut b, 0, 0x12, 0x55);
        write(&mut b, 1000, 0x0a, 0x30);
        assert_eq!(b.peek(0x12), None);
        assert_eq!(b.peek(0x0a), Some(0x30));
        write(&mut b, 1100, 0x12, 0x66);
        write(&mut b, 1100, 0x32, 0x66);
        assert!(!b.nonvolatile_busy());
        run(&mut b, t(1300));
        assert_eq!(b.peek(0x12), Some(162));
        assert_eq!(b.peek(0x0a), Some(0x10));
    }
    #[test]
    fn adc_self_test_uses_the_filter_and_membrane_test_completes_separately() {
        let mut b = Bma150::new(Time::ZERO);
        for (a, v) in [(0x14, 5), (0x0b, 1), (0x0c, 50), (0x0d, 0)] {
            write(&mut b, 0, a, v);
        }
        run(&mut b, t(4000));
        assert_eq!(b.peek(7), Some(64));
        write(&mut b, 4000, 0x0a, 8);
        run(&mut b, t(4334));
        assert_eq!(b.peek(7), Some(32));
        run(&mut b, t(4667));
        assert_eq!(b.peek(7), Some(0));
        run(&mut b, t(6000));
        assert_eq!(b.peek(9), Some(10));
        assert!(b.interrupt());
        write(&mut b, 6000, 0x0a, 4);
        run(&mut b, t(6200));
        assert_eq!(b.peek(0x0a), Some(4));
        assert_eq!(b.peek(9).unwrap() & 0x80, 0);
        run(&mut b, t(6334));
        assert_eq!(b.peek(0x0a), Some(0));
        assert_eq!(b.peek(9).unwrap() & 0x80, 0x80);
        assert_eq!(b.peek(7), Some(0)); // Diagnostic does not publish an invented deflection.
        run(&mut b, t(6667));
        assert_eq!(b.peek(7), Some(32));
    }
    #[test]
    fn automatic_sleep_has_four_pause_lengths_and_replays_mid_pause() {
        for (pause_code, ms) in [(0, 20), (1, 80), (2, 320), (3, 2560)] {
            let mut b = Bma150::new(Time::ZERO);
            for (a, v) in [
                (0x0b, 0),
                (0x14, 6),
                (0x15, 0x81 | (pause_code << 1)),
                (0x0a, 1),
            ] {
                write(&mut b, 0, a, v);
            }
            assert!(b.sleeping());
            assert_eq!(b.deadline(), Some(t(ms * 1000)));
            let mut restored = b.clone();
            run(&mut b, t(ms * 1000 - 1));
            assert!(b.sleeping());
            run(&mut b, t(ms * 1000));
            assert!(!b.sleeping());
            run(&mut b, t(ms * 1000 + 1334));
            assert!(b.sleeping());
            run(&mut restored, t(ms * 1000 + 1334));
            assert_eq!(b, restored);
            // The programmed sleep bit stayed one throughout the active check.
            assert_eq!(b.registers[0x0a], 1);
        }
    }
    fn freefall(latched: bool) -> Bma150 {
        let mut b = Bma150::new(Time::ZERO);
        b.set_input(Acceleration { x: 0, y: 0, z: 0 }).unwrap();
        for (a, v) in [
            (0x0b, 1),
            (0x0c, 50),
            (0x0d, 0),
            (0x14, 6),
            (0x15, if latched { 0x91 } else { 0x81 }),
            (0x0a, 1),
        ] {
            write(&mut b, 0, a, v);
        }
        b
    }
    #[test]
    fn latched_wake_waits_for_reset_and_nonlatched_irq_has_a_minimum_width() {
        let mut b = freefall(true);
        run(&mut b, t(30000));
        assert!(b.interrupt());
        assert!(!b.sleeping());
        b.set_input(Acceleration::STILL).unwrap();
        run(&mut b, t(31000));
        assert_eq!(b.peek(9), Some(8));
        assert!(b.interrupt());
        write(&mut b, 31000, 0x0a, 0x40);
        run(&mut b, t(32000));
        assert!(b.sleeping());
        assert!(!b.interrupt());
        assert_eq!(b.registers[0x0a] & 1, 0); // Actual sleep is independent of this bit.

        let mut b = freefall(false);
        let edge = loop {
            let at = b.deadline().unwrap();
            b.at_deadline(at, &mut ()).unwrap();
            if b.interrupt() {
                break at;
            }
            assert!(at < t(23000));
        };
        b.set_input(Acceleration {
            x: 1_000_000,
            y: 0,
            z: 0,
        })
        .unwrap();
        run(
            &mut b,
            edge.checked_add(Duration::from_micros(329)).unwrap(),
        );
        assert!(b.interrupt());
        assert!(!b.sleeping());
        assert_eq!(b.peek(9), Some(8));
        let mut restored = b.clone();
        let end = edge.checked_add(Duration::from_micros(330)).unwrap();
        run(&mut b, end);
        run(&mut restored, end);
        assert!(!b.interrupt());
        assert!(b.sleeping());
        assert_eq!(b, restored);
    }
}
