//! Timer B1: interval/reload operation, shared prescaler phase, and lazy reads.
use super::clocks::{Clocks, Tap};
use crate::{error::Error, time::Time};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerB1 {
    mode: u8,
    count: u8,
    load: u8,
    last: u64,
    enabled: bool,
}
impl Default for TimerB1 {
    fn default() -> Self {
        Self {
            mode: 0x38,
            count: 0,
            load: 0,
            last: 0,
            enabled: false,
        }
    }
}
impl TimerB1 {
    pub fn uses_watch(&self) -> bool {
        self.tap().source == super::clocks::Source::Watch
    }
    fn tap(&self) -> Tap {
        match self.mode & 7 {
            0 => Tap::system(8192),
            1 => Tap::system(2048),
            2 => Tap::system(256),
            3 => Tap::system(64),
            4 => Tap::system(16),
            5 => Tap::system(4),
            6 => Tap::watch(1024),
            _ => Tap::watch(256),
        }
    }
    pub fn sync(&mut self, now: Time, clocks: &Clocks) -> bool {
        let tick = clocks.ticks(now, self.tap());
        let n = tick.saturating_sub(self.last);
        self.last = tick;
        if !self.enabled || self.mode & 0x40 == 0 || n == 0 {
            return false;
        }
        let first = 256 - u64::from(self.count);
        if n < first {
            self.count = (u64::from(self.count) + n) as u8;
            return false;
        }
        let reload = if self.mode & 0x80 != 0 { self.load } else { 0 };
        let period = 256 - u64::from(reload);
        self.count = (u64::from(reload) + (n - first) % period) as u8;
        true
    }
    pub fn set_gate(&mut self, enabled: bool, now: Time, clocks: &Clocks) {
        self.enabled = enabled;
        self.last = clocks.ticks(now, self.tap());
    }
    pub fn read(&self, address: u16) -> u8 {
        if address == 0xf0d0 {
            self.mode
        } else {
            self.count
        }
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        if address == 0xf0d0 {
            if self.mode & 0x40 != 0 && (value ^ self.mode) & 0x87 != 0 {
                return Err(Error::Unsupported {
                    component: "Timer B1",
                    detail: "running mode/clock reconfiguration is not hardware-characterized",
                    address,
                });
            }
            self.mode = value | 0x38;
        } else {
            if self.mode & 0x40 != 0 {
                return Err(Error::Unsupported {
                    component: "Timer B1",
                    detail: "load while counting is not hardware-characterized",
                    address,
                });
            }
            self.load = value;
            self.count = value;
        }
        self.last = clocks.ticks(now, self.tap());
        Ok(())
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.enabled || self.mode & 0x40 == 0 {
            return Ok(None);
        }
        Ok(Some(clocks.edge(
            self.last + (256 - u64::from(self.count)),
            self.tap(),
        )?))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_ff_overflows_each_input_edge() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut t = TimerB1::default();
        t.set_gate(true, Time::ZERO, &c);
        t.write(0xf0d0, 0xbf, Time::ZERO, &c).unwrap();
        t.write(0xf0d1, 255, Time::ZERO, &c).unwrap();
        t.write(0xf0d0, 0xff, Time::ZERO, &c).unwrap();
        assert!(t.sync(c.edge(7, Tap::watch(256)).unwrap(), &c));
        assert_eq!(t.count, 255);
    }
    #[test]
    fn interval_load_also_sets_counter() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut t = TimerB1::default();
        t.write(0xf0d1, 254, Time::ZERO, &c).unwrap();
        assert_eq!(t.count, 254);
    }
}
