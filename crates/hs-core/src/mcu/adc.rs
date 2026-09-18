//! Successive approximation in 31 converter steps, with a held input sample
//! and a clocked ADTRG synchronizer.
use super::clocks::{ClockWait, Clocks, Tap};
use crate::{error::Error, time::Time};
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Phase {
    Sample = 0,
    Convert = 1,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Adc {
    mode: u8,
    control: u8,
    result: u16,
    sample: u16,
    next: Option<ClockWait>,
    phase: Phase,
    gate: bool,
    settling: Option<ClockWait>,
    trigger: bool,
    trigger_enabled: bool,
    trigger_rising: bool,
    pipeline: [bool; 2],
    trigger_next: Option<ClockWait>,
}
impl Default for Adc {
    fn default() -> Self {
        Self {
            mode: 0,
            control: 0x3f,
            result: 0,
            sample: 0,
            next: None,
            phase: Phase::Sample,
            gate: false,
            settling: None,
            trigger: false,
            trigger_enabled: false,
            trigger_rising: false,
            pipeline: [false; 2],
            trigger_next: None,
        }
    }
}
impl Adc {
    pub fn reset(&mut self) {
        let result = self.result;
        *self = Self::default();
        self.result = result;
    }
    pub fn peek(&self, a: u16) -> u8 {
        if a == 0xffbe {
            self.mode
        } else {
            self.control
        }
    }
    pub fn result(&self) -> u16 {
        self.result
    }
    pub fn channel(&self) -> u8 {
        self.mode & 15
    }
    pub fn uses_watch(&self) -> bool {
        self.mode & 0x30 == 0x30
    }
    fn tap(&self) -> Tap {
        match self.mode & 0x30 {
            0 => Tap::system(4),
            0x10 => Tap::system(2),
            0x20 => Tap::system(1),
            _ => Tap::watch(1),
        }
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        let convert = self
            .next
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(clocks))?;
        let trigger = self
            .trigger_next
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(clocks))?;
        Ok(convert.into_iter().chain(trigger).min())
    }
    pub fn sync(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self
            .settling
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(clocks))?
            .is_some_and(|at| at <= now)
        {
            self.settling = None;
        }
        Ok(())
    }
    fn start(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        if self.control & 0x80 != 0 {
            return Ok(());
        }
        self.control |= 0x80;
        self.phase = Phase::Sample;
        // Four converter steps for acquisition is the selected aperture;
        // 4+27 steps gives the documented maximum 124/62/31/31 states.
        let mut wait = ClockWait::after(now, 4, self.tap(), c)?;
        if !self.gate {
            wait.pause(now, c)?;
        }
        self.next = Some(wait);
        Ok(())
    }
    pub fn write(&mut self, a: u16, v: u8, now: Time, c: &Clocks) -> Result<(), Error> {
        self.sync(now, c)?;
        if a == 0xffbe {
            self.mode = v & 0x7f;
            let tap = self.tap();
            if let Some(wait) = &mut self.next {
                wait.select(now, tap, c)?;
            }
        } else {
            self.control = (self.control & 0x80) | (v & 0x40) | 0x3f;
            if v & 0x80 == 0 {
                self.control &= !0x80;
                self.next = None;
            } else {
                self.start(now, c)?;
            }
        }
        Ok(())
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, c: &Clocks) -> Result<(), Error> {
        if self.gate == gate {
            return Ok(());
        }
        self.sync(now, c)?;
        for wait in [&mut self.next, &mut self.trigger_next]
            .into_iter()
            .flatten()
        {
            if gate {
                wait.resume(now, c)?;
            } else {
                wait.pause(now, c)?;
            }
        }
        self.gate = gate;
        self.settling = if gate {
            Some(ClockWait::after(now, 10, Tap::cpu(), c)?)
        } else {
            None
        };
        self.schedule_trigger(now, c)
    }
    pub fn input_trigger(
        &mut self,
        selected: bool,
        high: bool,
        rising: bool,
        now: Time,
        c: &Clocks,
    ) -> Result<(), Error> {
        let enabled = selected && self.mode & 0x40 != 0;
        if !enabled || !self.trigger_enabled {
            self.pipeline = [high; 2];
            self.trigger_next = None;
        }
        self.trigger = high;
        self.trigger_rising = rising;
        self.trigger_enabled = enabled;
        self.schedule_trigger(now, c)
    }
    fn schedule_trigger(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        if self.gate
            && self.trigger_enabled
            && self.trigger_next.is_none()
            && self.pipeline.iter().any(|v| *v != self.trigger)
        {
            self.trigger_next = Some(ClockWait::after(now, 1, Tap::cpu(), c)?);
        }
        Ok(())
    }
    /// Return true only on result-register commit/IRRAD assertion. A coincident
    /// trigger sees the old ADSF; completing an active conversion wins that race.
    pub fn advance(
        &mut self,
        now: Time,
        analog_code: Option<u16>,
        c: &Clocks,
    ) -> Result<bool, Error> {
        if self.deadline(c)? != Some(now) {
            return Err(Error::Internal("ADC event at wrong timestamp"));
        }
        self.sync(now, c)?;
        if self
            .trigger_next
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(c))?
            == Some(now)
        {
            let old = self.pipeline[1];
            self.pipeline = [self.trigger, self.pipeline[0]];
            self.trigger_next = None;
            if self.trigger_enabled
                && old != self.pipeline[1]
                && self.pipeline[1] == self.trigger_rising
            {
                self.start(now, c)?;
            }
            self.schedule_trigger(now, c)?;
        }
        if self.next.as_ref().map_or(Ok(None), |w| w.deadline(c))? != Some(now) {
            return Ok(false);
        }
        match self.phase {
            Phase::Sample => {
                // An open mux leaves the capacitor charged to its previous
                // sample. A premature module-wake acquisition does likewise.
                if let Some(code) = analog_code.filter(|_| self.settling.is_none()) {
                    self.sample = code.min(1023);
                }
                self.phase = Phase::Convert;
                self.next = Some(ClockWait::after(now, 27, self.tap(), c)?);
                Ok(false)
            }
            Phase::Convert => {
                self.result = self.sample << 6;
                self.control &= !0x80;
                self.next = None;
                Ok(true)
            }
        }
    }
}

impl Adc {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        crate::state::require(
            self.mode & 0x80 == 0
                && self.control & 0x3f == 0x3f
                && self.result & 63 == 0
                && self.sample <= 1023,
            "invalid ADC latches",
        )?;
        for wait in [self.next, self.settling, self.trigger_next]
            .into_iter()
            .flatten()
        {
            wait.validate()?;
        }
        crate::state::require(
            self.next.is_none_or(|w| w.uses(self.tap()))
                && self.settling.is_none_or(|w| w.uses(Tap::cpu()))
                && self.trigger_next.is_none_or(|w| w.uses(Tap::cpu())),
            "invalid ADC clock source",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> (Clocks, Adc, Time) {
        let c = Clocks::new(
            Time::ZERO,
            super::super::clocks::Frequencies {
                main_hz: 1_000_000,
                watch_hz: 100_000,
                ..Default::default()
            },
        )
        .unwrap();
        let mut a = Adc::default();
        a.set_gate(true, Time::ZERO, &c).unwrap();
        (c, a, Time::from_micros(20))
    }
    #[test]
    fn every_clock_scales_acquisition_and_conversion_steps() {
        for (mode, sample, finish) in [
            (0x04, 36, 144),
            (0x14, 28, 82),
            (0x24, 24, 51),
            (0x34, 60, 330),
        ] {
            let (c, mut a, start) = ready();
            a.write(0xffbe, mode, start, &c).unwrap();
            a.write(0xffbf, 0x80, start, &c).unwrap();
            assert_eq!(a.deadline(&c).unwrap(), Some(Time::from_micros(sample)));
            assert!(!a.advance(Time::from_micros(sample), Some(321), &c).unwrap());
            assert_eq!(a.deadline(&c).unwrap(), Some(Time::from_micros(finish)));
            assert!(a.advance(Time::from_micros(finish), Some(900), &c).unwrap());
            assert_eq!(a.result(), 321 << 6);
        }
    }
    #[test]
    fn live_clock_changes_keep_remaining_work_and_open_mux_keeps_charge() {
        let (c, mut a, start) = ready();
        a.write(0xffbe, 0x24, start, &c).unwrap();
        a.write(0xffbf, 0x80, start, &c).unwrap();
        a.write(0xffbe, 0x35, Time::from_micros(22), &c).unwrap();
        assert_eq!(a.deadline(&c).unwrap(), Some(Time::from_micros(40)));
        assert_eq!(a.channel(), 5);
        a.advance(Time::from_micros(40), Some(700), &c).unwrap();
        a.write(0xffbe, 0x24, Time::from_micros(55), &c).unwrap();
        assert_eq!(a.deadline(&c).unwrap(), Some(Time::from_micros(81)));
        a.advance(Time::from_micros(81), Some(100), &c).unwrap();
        assert_eq!(a.result(), 700 << 6);
        a.write(0xffbe, 0, Time::from_micros(84), &c).unwrap();
        a.write(0xffbf, 0x80, Time::from_micros(84), &c).unwrap();
        while let Some(at) = a.deadline(&c).unwrap() {
            a.advance(at, None, &c).unwrap();
        }
        assert_eq!(a.result(), 700 << 6);
    }
    #[test]
    fn premature_power_up_capture_uses_the_existing_capacitor_charge() {
        let (c, mut a, _) = ready();
        a.write(0xffbe, 0x24, Time::ZERO, &c).unwrap();
        a.write(0xffbf, 0x80, Time::ZERO, &c).unwrap();
        while let Some(at) = a.deadline(&c).unwrap() {
            a.advance(at, Some(1023), &c).unwrap();
        }
        assert_eq!(a.result(), 0);
    }
    #[test]
    fn trigger_is_clocked_and_does_not_restart_an_active_conversion() {
        let (c, mut a, start) = ready();
        a.write(0xffbe, 0x64, start, &c).unwrap();
        a.input_trigger(true, false, true, start, &c).unwrap();
        a.input_trigger(true, true, true, Time::from_micros(23), &c)
            .unwrap();
        a.advance(Time::from_micros(24), None, &c).unwrap();
        assert_eq!(a.peek(0xffbf), 0x3f);
        a.advance(Time::from_micros(25), None, &c).unwrap();
        assert_eq!(a.peek(0xffbf), 0xbf);
        a.input_trigger(true, false, true, Time::from_micros(26), &c)
            .unwrap();
        for us in [27, 28, 29] {
            a.advance(Time::from_micros(us), Some(600), &c).unwrap();
        }
        a.input_trigger(true, true, true, Time::from_micros(30), &c)
            .unwrap();
        for us in [31, 32] {
            a.advance(Time::from_micros(us), Some(900), &c).unwrap();
        }
        assert_eq!(a.deadline(&c).unwrap(), Some(Time::from_micros(56)));
        assert!(a.advance(Time::from_micros(56), Some(900), &c).unwrap());
        assert_eq!(a.result(), 600 << 6);
    }
    #[test]
    fn changing_input_after_aperture_does_not_change_held_sample() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut a = Adc::default();
        a.set_gate(true, Time::ZERO, &c).unwrap();
        let start = c.after(Time::ZERO, 10, Tap::cpu()).unwrap();
        a.write(0xffbe, 0x27, start, &c).unwrap();
        a.write(0xffbf, 0xbf, start, &c).unwrap();
        assert!(!a
            .advance(a.deadline(&c).unwrap().unwrap(), Some(500), &c)
            .unwrap());
        assert!(a
            .advance(a.deadline(&c).unwrap().unwrap(), Some(900), &c)
            .unwrap());
        assert_eq!(a.result(), 500 << 6);
        a.reset();
        assert_eq!(a.result(), 500 << 6);
        assert_eq!((a.peek(0xffbe), a.peek(0xffbf)), (0, 0x3f));
    }
}
