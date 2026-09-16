//! RTC and its alternate free-running counter. Busy entry/exit are distinct
//! from the second rollover; default busy placement is a documented 62.5-ms
//! duration with an explicitly unmeasured phase witness (see STATUS.md).
use super::clocks::{Clocks, Tap};
use crate::{error::Error, time::Time};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rtc {
    pub flags: u8,
    data: [u8; 4],
    control1: u8,
    control2: u8,
    source: u8,
    phase: u16,
    last: u64,
    enabled: bool,
    busy: bool,
    pending: u8,
}
impl Default for Rtc {
    fn default() -> Self {
        Self {
            flags: 0,
            data: [0; 4],
            control1: 0,
            control2: 0,
            source: 8,
            phase: 0,
            last: 0,
            enabled: true,
            busy: false,
            pending: 0,
        }
    }
}
impl Rtc {
    pub fn uses_watch(&self) -> bool {
        self.tap().source == super::clocks::Source::Watch
    }
    fn tap(&self) -> Tap {
        match self.source & 15 {
            0 => Tap::system(8),
            1 => Tap::system(32),
            2 => Tap::system(128),
            3 => Tap::system(256),
            4 => Tap::system(512),
            5 => Tap::system(2048),
            6 => Tap::system(4096),
            7 => Tap::system(8192),
            _ => Tap::watch(4),
        }
    }
    fn running(&self) -> bool {
        self.enabled && self.control1 & 0x90 == 0x80
    }
    fn distance(&self) -> u64 {
        if self.source & 15 != 8 {
            return 256 - u64::from(self.data[0]);
        }
        let p = u64::from(self.phase);
        let quarter = ((p / 2048) + 1) * 2048;
        quarter.min(if p < 7680 { 7680 } else { 8192 }) - p
    }
    pub fn sync(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        let tick = clocks.ticks(now, self.tap());
        let mut n = tick.saturating_sub(self.last);
        self.last = tick;
        if !self.running() {
            return Ok(());
        }
        if self.source & 15 != 8 {
            if n >= 256 - u64::from(self.data[0]) {
                self.flags |= self.control2 & 0x80;
            }
            self.data[0] = self.data[0].wrapping_add(n as u8);
            return Ok(());
        }
        while n != 0 {
            let d = self.distance();
            let advance = n.min(d);
            n -= advance;
            self.phase += advance as u16;
            if advance != d {
                continue;
            }
            if self.phase == 7680 {
                self.busy = true;
                self.pending = self.rollover_flags();
                if self.control1 & 8 == 0 {
                    self.flags |= self.pending & self.control2;
                }
            }
            if self.phase % 2048 == 0 {
                self.flags |= self.control2 & 1;
                if self.phase % 4096 == 0 {
                    self.flags |= self.control2 & 2;
                }
            }
            if self.phase == 8192 {
                self.phase = 0;
                self.advance_second()?;
                if self.control1 & 8 != 0 {
                    self.flags |= self.pending & self.control2;
                }
                self.pending = 0;
                self.busy = false;
            }
        }
        Ok(())
    }
    fn rollover_flags(&self) -> u8 {
        let mut flags = 4;
        if self.data[0] == 0x59 {
            flags |= 8;
            if self.data[1] == 0x59 {
                flags |= 0x10;
                if (self.control1 & 0x40 != 0 && self.data[2] == 0x23)
                    || (self.control1 & 0x40 == 0
                        && self.data[2] == 0x11
                        && self.control1 & 0x20 != 0)
                {
                    flags |= 0x20;
                    if self.data[3] == 6 {
                        flags |= 0x40;
                    }
                }
            }
        }
        flags
    }
    fn advance_second(&mut self) -> Result<(), Error> {
        let limits = [
            0x59,
            0x59,
            if self.control1 & 0x40 != 0 {
                0x23
            } else {
                0x11
            },
        ];
        for (i, limit) in limits.into_iter().enumerate() {
            if self.data[i] > limit || self.data[i] & 15 > 9 {
                return Err(Error::Unsupported {
                    component: "RTC",
                    detail: "invalid BCD rollover is not characterized",
                    address: 0xf068 + i as u16,
                });
            }
            if self.data[i] == limit {
                self.data[i] = 0;
                if i == 2 && self.control1 & 0x40 == 0 {
                    self.control1 ^= 0x20;
                    if self.control1 & 0x20 != 0 {
                        return Ok(());
                    }
                }
            } else {
                self.data[i] += 1;
                if self.data[i] & 15 == 10 {
                    self.data[i] += 6;
                }
                return Ok(());
            }
        }
        self.data[3] = (self.data[3] + 1) % 7;
        Ok(())
    }
    pub fn set_gate(&mut self, enabled: bool, now: Time, clocks: &Clocks) {
        self.enabled = enabled;
        self.last = clocks.ticks(now, self.tap());
    }
    pub fn read(&self, address: u16) -> u8 {
        match address {
            0xf067 => self.flags,
            0xf068..=0xf06b => {
                self.data[usize::from(address - 0xf068)]
                    | if self.source & 15 == 8 && self.busy {
                        0x80
                    } else {
                        0
                    }
            }
            0xf06c => self.control1,
            0xf06d => self.control2,
            0xf06f => self.source,
            _ => 0,
        }
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        match address {
            0xf067 => self.flags &= value,
            0xf068..=0xf06b => {
                if self.control1 & 0x80 != 0 {
                    return Err(Error::Unsupported {
                        component: "RTC",
                        detail: "time write while RUN is set",
                        address,
                    });
                }
                let i = usize::from(address - 0xf068);
                self.data[i] = value & [0x7f, 0x7f, 0x3f, 7][i];
            }
            0xf06c => {
                if value & 0x10 != 0 {
                    self.data = [0; 4];
                    self.phase = 0;
                    self.flags = 0;
                    self.control2 = 0;
                    self.control1 = 0x10;
                    self.busy = false;
                    self.pending = 0;
                } else {
                    self.control1 = value & 0xf8;
                }
            }
            0xf06d => self.control2 = value,
            0xf06f => {
                if value & 15 > 8 {
                    return Err(Error::Unsupported {
                        component: "RTC",
                        detail: "prohibited clock source",
                        address,
                    });
                }
                self.source = value & 0x7f;
            }
            _ => {
                return Err(Error::Unmapped {
                    address,
                    write: true,
                    width: 1,
                })
            }
        }
        self.last = clocks.ticks(now, self.tap());
        Ok(())
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.running() {
            return Ok(None);
        }
        Ok(Some(clocks.edge(self.last + self.distance(), self.tap())?))
    }
    pub fn interrupt(&self) -> Option<u8> {
        let p = self.flags & self.control2;
        (p != 0).then(|| 23 + p.trailing_zeros() as u8)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn running() -> (Rtc, Clocks) {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut r = Rtc::default();
        r.write(0xf06c, 0x10, Time::ZERO, &c).unwrap();
        r.write(0xf06c, 0, Time::ZERO, &c).unwrap();
        r.write(0xf06d, 0x7f, Time::ZERO, &c).unwrap();
        r.write(0xf06c, 0xc8, Time::ZERO, &c).unwrap();
        (r, c)
    }
    #[test]
    fn busy_and_rollover_are_separate() {
        let (mut r, c) = running();
        r.sync(Time::from_micros(937_500), &c).unwrap();
        assert_eq!(r.read(0xf068), 0x80);
        assert_eq!(r.flags & 4, 0);
        r.sync(Time::from_micros(1_000_000), &c).unwrap();
        assert_eq!(r.read(0xf068), 1);
        assert_ne!(r.flags & 4, 0);
    }
    #[test]
    fn partition_does_not_change_calendar() {
        let (mut a, c) = running();
        let mut b = a.clone();
        a.sync(Time::from_micros(3_234_567), &c).unwrap();
        for i in 1..=100 {
            b.sync(Time::from_micros(i * 32_345), &c).unwrap();
        }
        b.sync(Time::from_micros(3_234_567), &c).unwrap();
        assert_eq!(a, b);
    }
}
