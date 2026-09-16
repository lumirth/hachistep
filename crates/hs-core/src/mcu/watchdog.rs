//! Watchdog owner. MOV-qualified write-inhibit bits and the reset-cause bit
//! are modeled independently of the interrupt controller.
use crate::{error::Error, time::Time};
use super::clocks::{Clocks, Tap};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watchdog { mode: u8, control1: u8, control2: u8, count: u8, last: u64, seen_overflow: bool, gate: bool }
impl Default for Watchdog {
    fn default() -> Self { Self { mode: 0xf0, control1: 0xae, control2: 0x57, count: 0, last: 0, seen_overflow: false, gate: true } }
}
impl Watchdog {
    fn tap(&self) -> Tap {
        match self.mode & 15 {
            0..=3 => Tap::on_chip(2048), 4 => Tap::watch(16), 5 => Tap::watch(256),
            v => Tap::system(64u32 << (v.saturating_sub(8))),
        }
    }
    fn running(&self) -> bool { self.control1 & 4 != 0 }
    pub fn sync(&mut self, now: Time, clocks: &Clocks) -> bool {
        let tick = clocks.ticks(now, self.tap()); let n = tick.saturating_sub(self.last); self.last = tick;
        if !self.running() || n == 0 { return false; }
        let overflow = n >= 256 - u64::from(self.count);
        self.count = self.count.wrapping_add(n as u8);
        if overflow {
            self.control2 |= 0x80;
            if self.control2 & 0x20 == 0 { self.control1 |= 1; return true; }
        }
        false
    }
    pub fn read(&mut self, address: u16) -> u8 {
        if address == 0xffb2 { self.seen_overflow = self.control2 & 0x80 != 0; }
        self.peek(address)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address { 0xffb0 => self.mode, 0xffb1 => self.control1,
            0xffb2 => self.control2, _ => self.count }
    }
    pub fn write(&mut self, address: u16, value: u8, mov_byte: bool, now: Time, clocks: &Clocks) -> Result<(), Error> {
        match address {
            0xffb0 => {
                if matches!(value & 15, 6 | 7) { return Err(Error::Unsupported { component: "WDT", detail: "reserved watchdog clock", address }); }
                self.mode = value | 0xf0;
            }
            0xffb1 if mov_byte => {
                let old = self.control1;
                if value & 0x80 == 0 { self.control1 = (self.control1 & !0x40) | (value & 0x40); }
                if value & 0x20 == 0 { self.control1 = (self.control1 & !0x10) | (value & 0x10); }
                if old & 0x10 != 0 {
                    if value & 8 == 0 { self.control1 = (self.control1 & !4) | (value & 4); }
                    if value & 2 == 0 && value & 1 == 0 { self.control1 &= !1; }
                }
                self.control1 |= 0xaa;
            }
            0xffb2 if mov_byte => {
                if value & 0x80 == 0 && self.seen_overflow { self.control2 &= !0x80; self.seen_overflow = false; }
                if value & 0x40 == 0 { self.control2 = (self.control2 & !0x20) | (value & 0x20); }
                if value & 0x10 == 0 { self.control2 = (self.control2 & !8) | (value & 8); }
                self.control2 |= 0x57;
            }
            0xffb3 if self.control1 & 0x40 != 0 => self.count = value,
            _ => {}
        }
        self.last = clocks.ticks(now, self.tap()); Ok(())
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, clocks: &Clocks) {
        self.gate = gate; self.last = clocks.ticks(now, self.tap());
    }
    pub fn reset(&mut self, watchdog: bool, now: Time, clocks: &Clocks) {
        *self = Self::default(); if watchdog { self.control1 |= 1; }
        self.last = clocks.ticks(now, self.tap());
    }
    pub fn interrupt(&self) -> bool { self.control2 & 0xa8 == 0xa8 }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.running() { return Ok(None); }
        Ok(Some(clocks.edge(self.last + 256 - u64::from(self.count), self.tap())?))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn watchdog_write_requires_two_qualified_accesses() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap(); let mut w = Watchdog::default();
        w.write(0xffb1, 0, true, Time::ZERO, &c).unwrap(); assert_ne!(w.control1 & 4, 0);
        w.write(0xffb1, 0x10, false, Time::ZERO, &c).unwrap(); assert_eq!(w.control1 & 0x10, 0);
        w.write(0xffb1, 0x10, true, Time::ZERO, &c).unwrap();
        w.write(0xffb1, 0, true, Time::ZERO, &c).unwrap(); assert_eq!(w.control1 & 4, 0);
    }
}
