//! Timer W counter/compare/PWM owner. Input capture and buffering are explicit
//! unsupported boundaries in this starter, not silently simulated as compare.
use crate::{error::Error, time::Time};
use super::clocks::{Clocks, Tap};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerW {
    mode: u8, control: u8, enable: u8, status: u8, io: [u8; 2],
    count: u16, general: [u16; 4], output: u8, seen: u8,
    last: u64, gate: bool,
}
impl Default for TimerW {
    fn default() -> Self { Self { mode: 0x48, control: 0, enable: 0x70, status: 0x70,
        io: [0x88; 2], count: 0, general: [0xffff; 4], output: 0, seen: 0, last: 0, gate: false } }
}
impl TimerW {
    pub fn uses_watch(&self) -> bool { self.tap().source == super::clocks::Source::Watch }
    fn tap(&self) -> Tap {
        match self.control >> 4 & 7 { 0 => Tap::system(1), 1 => Tap::system(2),
            2 => Tap::system(4), 3 => Tap::system(8), 4 => Tap::watch(1),
            5 => Tap::watch(4), _ => Tap::watch(16) }
    }
    fn running(&self) -> bool { self.gate && self.mode & 0x80 != 0 }
    fn distance(&self) -> u64 {
        let mut d = 65536 - u64::from(self.count);
        for value in self.general { d = d.min(u64::from(value.wrapping_sub(self.count)) + 1); }
        d
    }
    /// Advance the one canonical counter recurrence through consequential
    /// boundaries. There is no tick executor beside a bulk executor.
    pub fn sync(&mut self, now: Time, clocks: &Clocks) {
        let tick = clocks.ticks(now, self.tap()); let mut n = tick.saturating_sub(self.last); self.last = tick;
        if !self.running() { return; }
        while n != 0 {
            let d = self.distance(); let k = n.min(d); let old = self.count;
            self.count = self.count.wrapping_add(k as u16); n -= k;
            if k != d { continue; }
            let mut matches = 0u8;
            for (i, value) in self.general.into_iter().enumerate() {
                if u64::from(value.wrapping_sub(old)) + 1 == k { matches |= 1 << i; }
            }
            if 65536 - u64::from(old) == k { self.status |= 0x80; }
            self.status |= matches;
            for i in 0..4 {
                if matches & (1 << i) != 0 {
                    let mode = (self.io[i / 2] >> ((i % 2) * 4)) & 7;
                    if i > 0 && self.mode & (1 << (i - 1)) != 0 {
                        // In PWM, the programmed initial polarity is restored
                        // at compare A, and inverted at the channel compare.
                        let initial = self.control & (1 << i) != 0;
                        self.set_output(i, !initial);
                    } else {
                        match mode { 1 => self.set_output(i, false), 2 => self.set_output(i, true),
                            3 => self.output ^= 1 << i, _ => {} }
                    }
                }
            }
            if matches & 1 != 0 {
                if self.control & 0x80 != 0 { self.count = 0; }
                for i in 1..4 {
                    if self.mode & (1 << (i - 1)) != 0 { self.set_output(i, self.control & (1 << i) != 0); }
                }
            }
        }
    }
    fn set_output(&mut self, channel: usize, high: bool) {
        if high { self.output |= 1 << channel; } else { self.output &= !(1 << channel); }
    }
    pub fn outputs(&self) -> u8 { self.output }
    pub fn drives(&self) -> u8 {
        let mut mask = 0;
        for i in 0..4 {
            let mode = self.io[i / 2] >> ((i % 2) * 4) & 7;
            if matches!(mode, 1..=3) || (i > 0 && self.mode & (1 << (i - 1)) != 0) { mask |= 1 << i; }
        }
        mask
    }
    pub fn interrupt(&self) -> bool { self.status & self.enable & 0x8f != 0 }
    pub fn read(&mut self, address: u16) -> u8 {
        if address == 0xf0f3 { self.seen = self.status & 0x8f; }
        self.peek(address)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address { 0xf0f0 => self.mode, 0xf0f1 => self.control, 0xf0f2 => self.enable,
            0xf0f3 => self.status, 0xf0f4..=0xf0f5 => self.io[usize::from(address - 0xf0f4)],
            _ => { let v = self.word(address & !1); if address & 1 == 0 { (v >> 8) as u8 } else { v as u8 } } }
    }
    pub fn word(&self, address: u16) -> u16 {
        if address == 0xf0f6 { self.count } else { self.general[usize::from((address - 0xf0f8) / 2)] }
    }
    pub fn write_word(&mut self, address: u16, value: u16, now: Time, clocks: &Clocks) {
        if address == 0xf0f6 { self.count = value; } else { self.general[usize::from((address - 0xf0f8) / 2)] = value; }
        self.last = clocks.ticks(now, self.tap());
    }
    pub fn write(&mut self, address: u16, value: u8, now: Time, clocks: &Clocks) -> Result<(), Error> {
        match address {
            0xf0f0 => {
                if value & 0x30 != 0 { return Err(Error::Unsupported { component: "Timer W", detail: "buffer transfer modes need implementation", address }); }
                self.mode = value | 0x48;
            }
            0xf0f1 => {
                if value & 0x70 == 0x70 { return Err(Error::Unsupported { component: "Timer W", detail: "external FTCI clock needs package routing", address }); }
                self.control = value; self.output = value & 15;
            }
            0xf0f2 => self.enable = value | 0x70,
            0xf0f3 => { self.status &= !(self.seen & !value); self.seen &= value; self.status |= 0x70; }
            0xf0f4..=0xf0f5 => {
                if value & 4 != 0 || value & 0x40 != 0 { return Err(Error::Unsupported { component: "Timer W", detail: "input capture needs its sampled pin pipeline", address }); }
                self.io[usize::from(address - 0xf0f4)] = value | 0x88;
            }
            _ => {
                return Err(Error::Unsupported { component: "Timer W", detail: "byte lane access to a 16-bit timer register is not yet characterized", address });
            }
        }
        self.last = clocks.ticks(now, self.tap()); Ok(())
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, clocks: &Clocks) { self.gate = gate; self.last = clocks.ticks(now, self.tap()); }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.running() { return Ok(None); }
        Ok(Some(clocks.edge(self.last + self.distance(), self.tap())?))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compare_occurs_on_leaving_the_matching_count() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap(); let mut w = TimerW::default();
        w.set_gate(true, Time::ZERO, &c); w.write_word(0xf0f8, 2, Time::ZERO, &c);
        w.write(0xf0f1, 0x80, Time::ZERO, &c).unwrap(); w.write(0xf0f0, 0x80, Time::ZERO, &c).unwrap();
        w.sync(c.edge(2, Tap::system(1)).unwrap(), &c); assert_eq!(w.count, 2); assert_eq!(w.status & 1, 0);
        w.sync(c.edge(3, Tap::system(1)).unwrap(), &c); assert_eq!(w.count, 0); assert_eq!(w.status & 1, 1);
    }
}
