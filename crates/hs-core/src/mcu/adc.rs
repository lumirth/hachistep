//! ADC sample aperture and completion are distinct appointments. The nominal
//! transfer function is supplied by the board, never by retail save data.
use super::clocks::{ClockWait, Clocks, Tap};
use crate::{error::Error, time::Time};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Sample,
    Convert,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adc {
    mode: u8,
    control: u8,
    result: u16,
    sample: u16,
    next: Option<ClockWait>,
    phase: Phase,
    finish: Option<ClockWait>,
    gate: bool,
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
            finish: None,
            gate: false,
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
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        self.next
            .as_ref()
            .map_or(Ok(None), |wait| wait.deadline(clocks))
    }
    pub fn uses_watch(&self) -> bool {
        self.mode & 0x30 == 0x30
    }
    fn start(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        if !(4..=9).contains(&self.channel()) {
            return Err(Error::Unsupported {
                component: "ADC",
                detail: "conversion without a selected channel",
                address: 0xffbe,
            });
        }
        let (tap, cycles) = match self.mode & 0x30 {
            0 => (Tap::system(1), 124),
            0x10 => (Tap::system(1), 62),
            0x20 => (Tap::system(1), 31),
            _ => (Tap::watch(1), 31),
        };
        self.control |= 0x80;
        self.phase = Phase::Sample;
        // Aperture at four reference edges is an explicit model witness; the
        // datasheet bounds total conversion, not this subphase placement.
        self.next = Some(ClockWait::after(now, 4, tap, c)?);
        self.finish = Some(ClockWait::after(now, cycles, tap, c)?);
        if !self.gate {
            for wait in [&mut self.next, &mut self.finish].into_iter().flatten() {
                wait.pause(now, c)?;
            }
        }
        Ok(())
    }
    pub fn write(&mut self, a: u16, v: u8, now: Time, c: &Clocks) -> Result<(), Error> {
        if a == 0xffbe {
            if self.control & 0x80 != 0 && self.mode != (v & 0x7f) {
                return Err(Error::Unsupported {
                    component: "ADC",
                    detail: "channel or clock changed during conversion",
                    address: a,
                });
            }
            if v & 0x40 != 0 {
                return Err(Error::Unsupported {
                    component: "ADC",
                    detail: "external ADTRG synchronization is not implemented",
                    address: a,
                });
            }
            self.mode = v & 0x7f;
        } else {
            let was = self.control & 0x80 != 0;
            self.control = (v & 0xc0) | 0x3f;
            if v & 0x80 == 0 {
                self.next = None;
                self.finish = None;
            } else if !was {
                self.start(now, c)?;
            }
        }
        Ok(())
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.gate == gate {
            return Ok(());
        }
        for wait in [&mut self.next, &mut self.finish].into_iter().flatten() {
            if gate {
                wait.resume(now, clocks)?;
            } else {
                wait.pause(now, clocks)?;
            }
        }
        self.gate = gate;
        Ok(())
    }
    /// Return true only on result-register commit/IRRAD assertion.
    pub fn advance(&mut self, now: Time, analog_code: u16, clocks: &Clocks) -> Result<bool, Error> {
        if self.deadline(clocks)? != Some(now) {
            return Err(Error::Internal("ADC event at wrong timestamp"));
        }
        match self.phase {
            Phase::Sample => {
                self.sample = analog_code.min(1023);
                self.phase = Phase::Convert;
                self.next = self.finish.take();
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_input_after_aperture_does_not_change_held_sample() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut a = Adc::default();
        a.set_gate(true, Time::ZERO, &c).unwrap();
        a.write(0xffbe, 0x27, Time::ZERO, &c).unwrap();
        a.write(0xffbf, 0xbf, Time::ZERO, &c).unwrap();
        assert!(!a
            .advance(a.deadline(&c).unwrap().unwrap(), 500, &c)
            .unwrap());
        assert!(a
            .advance(a.deadline(&c).unwrap().unwrap(), 900, &c)
            .unwrap());
        assert_eq!(a.result(), 500 << 6);
        a.reset();
        assert_eq!(a.result(), 500 << 6);
        assert_eq!((a.peek(0xffbe), a.peek(0xffbf)), (0, 0x3f));
    }
}
