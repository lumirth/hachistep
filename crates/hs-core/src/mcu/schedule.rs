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
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            self.slots[i] = if 1 << i == SSU {
                serial
            } else {
                mcu.appointment(1 << i)?
            };
        }
        self.next = self.slots.iter().flatten().copied().min();
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
        if !self.startup.supplied() {
            return Ok(false);
        }
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
            self.apply_gates(now, out)?;
        }
        Ok(reset)
    }
    pub(crate) fn access_peripherals(a: u16, write: bool) -> u16 {
        // These writes can reselect or gate shared clock consumers. Settle the
        // old interval before apply_gates rebases any consumer's phase.
        if write
            && (Control::handles(a)
                || matches!(
                    a,
                    0xf06f | 0xf0d0 | 0xf0e2 | 0xf0f1 | 0xffb0..=0xffb3 | 0xffbe | 0xf022
                )
                || Aec::handles(a))
        {
            return ALL;
        }
        if Sci::handles(a) {
            return SCI;
        }
        if Aec::handles(a) || (0xff8c..=0xff8f).contains(&a) {
            return AEC;
        }
        match a {
            0xf067..=0xf06f => RTC,
            0xf0d0..=0xf0d1 => TIMER_B1,
            0xf0dc..=0xf0de => COMPARATORS,
            0xf0f0..=0xf0ff => TIMER_W,
            0xffb0..=0xffb3 => WATCHDOG,
            0xffbc..=0xffbf => ADC,
            0xf0e0..=0xf0eb => SSU,
            0xf078..=0xf07f => IIC,
            _ => 0,
        }
    }
}
