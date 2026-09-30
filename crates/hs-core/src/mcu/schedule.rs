//! MCU appointments retain the identity of the component that needs service.
//! Counter projection and register access synchronize only the affected owners.
use super::*;

pub(crate) const STARTUP: u16 = 1 << 0;
pub(crate) const RTC: u16 = 1 << 1;
pub(crate) const TIMER_B1: u16 = 1 << 2;
pub(crate) const TIMER_W: u16 = 1 << 3;
pub(crate) const WATCHDOG: u16 = 1 << 4;
pub(crate) const SSU: u16 = 1 << 5;
pub(crate) const SCI: u16 = 1 << 6;
pub(crate) const IIC: u16 = 1 << 7;
pub(crate) const ADC: u16 = 1 << 8;
pub(crate) const COMPARATORS: u16 = 1 << 9;
pub(crate) const AEC: u16 = 1 << 10;
pub(crate) const ALL: u16 = (1 << 11) - 1;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Appointments {
    slots: [Option<Time>; 11],
    next: Option<Time>,
}
impl Appointments {
    pub fn next(&self) -> Option<Time> {
        self.next
    }
    pub fn due(&self, now: Time) -> u16 {
        self.slots
            .iter()
            .enumerate()
            .fold(0, |mask, (i, at)| mask | (u16::from(*at == Some(now)) << i))
    }
    pub fn update(&mut self, mut mask: u16, mcu: &Mcu, serial: Option<Time>) -> Result<(), Error> {
        if mask == 0 {
            return Ok(());
        }
        let mut minimum_removed = false;
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let before = self.slots[i];
            let after = if 1 << i == SSU {
                serial
            } else {
                mcu.appointment(1 << i)?
            };
            self.slots[i] = after;
            minimum_removed |= before.is_some()
                && before == self.next
                && after.is_none_or(|at| at > before.unwrap());
            if let Some(at) = after {
                self.next = Some(self.next.map_or(at, |next| next.min(at)));
            }
        }
        // Moving an unrelated slot cannot invalidate the cached minimum.
        if minimum_removed {
            self.next = self.slots.iter().flatten().copied().min();
        }
        Ok(())
    }
}

impl Mcu {
    pub fn deadline(&self) -> Result<Option<Time>, Error> {
        let mut next = None;
        for i in 0..11 {
            if let Some(at) = self.appointment(1 << i)? {
                next = Some(next.map_or(at, |old: Time| old.min(at)));
            }
        }
        Ok(next)
    }
    fn appointment(&self, owner: u16) -> Result<Option<Time>, Error> {
        if !self.startup.supplied() {
            return Ok(None);
        }
        match owner {
            STARTUP => Ok(self.startup.deadline()),
            RTC => self.rtc.deadline(&self.clocks),
            TIMER_B1 => self.timer_b1.deadline(&self.clocks),
            TIMER_W => self.timer_w.deadline(&self.clocks),
            WATCHDOG => self.watchdog.deadline(&self.clocks),
            SSU => self.ssu.deadline(&self.clocks),
            SCI => self.sci.deadline(&self.clocks),
            IIC => self.iic.deadline(&self.clocks),
            ADC => self.adc.deadline(&self.clocks),
            COMPARATORS => Ok(self.comparators.deadline()),
            AEC => self.aec.deadline(&self.clocks),
            _ => Err(Error::Internal("invalid MCU appointment owner")),
        }
    }
    pub(crate) fn sync_peripherals(
        &mut self,
        mask: u16,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<bool, Error> {
        self.sync_peripherals_at(mask, || Ok(now), out)
    }
    pub(crate) fn sync_peripherals_at(
        &mut self,
        mask: u16,
        now: impl FnOnce() -> Result<Time, Error>,
        out: &mut dyn Output,
    ) -> Result<bool, Error> {
        #[cfg(feature = "profile-work")]
        self.work.calls.add(1);
        // SSU and IIC advance at their interaction appointments through the
        // physical network. An owner identity alone is not a clock dependency.
        if !self.startup.supplied() || mask & !(SSU | IIC) == 0 {
            return Ok(false);
        }
        let now = now()?;
        #[cfg(feature = "profile-work")]
        self.work.owners.add(u64::from(
            (mask & (RTC | TIMER_B1 | TIMER_W | WATCHDOG | SCI | ADC | COMPARATORS | AEC))
                .count_ones(),
        ));
        if mask & SCI != 0 {
            self.sci.sync(now, &self.clocks)?;
        }
        if mask & ADC != 0 {
            self.adc.sync(now, &self.clocks)?;
        }
        if mask & COMPARATORS != 0 {
            self.comparators.sync(now)?;
        }
        if mask & AEC != 0 {
            self.aec.sync(now, &self.clocks)?;
            self.collect_aec_requests();
        }
        if mask & RTC != 0 {
            self.rtc.sync(now, &self.clocks)?;
        }
        if mask & TIMER_B1 != 0 && self.timer_b1.sync(now, &self.clocks) {
            self.control.irr2 |= 4;
        }
        if mask & TIMER_W != 0 {
            self.timer_w.sync(now, &self.clocks)?;
        }
        let reset = mask & WATCHDOG != 0 && self.watchdog.sync(now, &self.clocks);
        if mask & STARTUP != 0 && self.startup.deadline() == Some(now) {
            #[cfg(feature = "profile-work")]
            self.work.owners.add(1);
            self.apply_gates(now, out)?;
        }
        Ok(reset)
    }
}
