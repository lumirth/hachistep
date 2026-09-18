//! WDT counter, private ROSC divider, qualified control writes, and reset cause.
//! Reset assertion is returned to the machine, which owns the 512-ROSC hold.
use super::clocks::{Clocks, Source, Tap};
use crate::{cpu::WriteOrigin, error::Error, time::Time};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watchdog {
    mode: u8,
    control1: u8,
    control2: u8,
    count: u8,
    last: u64,
    rosc_last: u64,
    rosc_ticks: u64,
    rosc_phase: u16,
    seen_overflow: bool,
    gate: bool,
    source_available: bool,
    reset_held: bool,
}
impl Default for Watchdog {
    fn default() -> Self {
        Self {
            mode: 0xf0,
            control1: 0xae,
            control2: 0x57,
            count: 0,
            last: 0,
            rosc_last: 0,
            rosc_ticks: 0,
            rosc_phase: 0,
            seen_overflow: false,
            gate: true,
            source_available: true,
            reset_held: false,
        }
    }
}
impl Watchdog {
    pub fn source(&self) -> Source {
        match self.mode & 15 {
            4 | 5 => Source::Watch,
            8..=15 => Source::System,
            _ => Source::OnChip,
        }
    }
    fn tap(&self) -> Option<Tap> {
        match self.mode & 15 {
            0..=3 => Some(Tap::on_chip(2048)),
            4 => Some(Tap::watch(16)),
            5 => Some(Tap::watch(256)),
            6 | 7 => None,
            n => Some(Tap::system(64 << (n - 8))),
        }
    }
    fn running(&self) -> bool {
        self.control1 & 4 != 0 && self.source_available && !self.reset_held && self.tap().is_some()
    }
    pub fn rosc_required(&self) -> bool {
        !self.reset_held && self.rosc_for_module(self.gate)
    }
    pub fn rosc_for_module(&self, gate: bool) -> bool {
        self.mode & 15 <= 3 && (gate || self.control1 & 4 != 0)
    }
    fn tick(&self, now: Time, clocks: &Clocks) -> u64 {
        if self.mode & 15 <= 3 {
            self.rosc_ticks
        } else {
            self.tap().map_or(0, |tap| clocks.ticks(now, tap))
        }
    }
    pub fn sync(&mut self, now: Time, clocks: &Clocks) -> bool {
        let raw = clocks.ticks(now, Tap::on_chip(1));
        if self.rosc_required() {
            let cycles = raw.saturating_sub(self.rosc_last) + u64::from(self.rosc_phase);
            self.rosc_ticks += cycles / 2048;
            self.rosc_phase = (cycles % 2048) as u16;
        }
        self.rosc_last = raw;
        let tick = self.tick(now, clocks);
        let n = tick.saturating_sub(self.last);
        self.last = tick;
        if !self.running() || n == 0 {
            return false;
        }
        let overflow = n >= 256 - u64::from(self.count);
        self.count = self.count.wrapping_add(n as u8);
        if overflow {
            self.control2 |= 0x80;
            if self.control2 & 0x20 == 0 {
                self.control1 |= 1;
                return true;
            }
        }
        false
    }
    pub fn read(&mut self, address: u16) -> u8 {
        if address == 0xffb2 {
            self.seen_overflow = self.control2 & 0x80 != 0;
        }
        self.peek(address)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0xffb0 => self.mode,
            0xffb1 => self.control1,
            0xffb2 => self.control2,
            _ => self.count,
        }
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        origin: WriteOrigin,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        match address {
            0xffb0 => self.mode = value | 0xf0,
            0xffb1 if origin.is_mov() => {
                let old = self.control1;
                if value & 0x80 == 0 {
                    self.control1 = (self.control1 & !0x40) | (value & 0x40);
                }
                if value & 0x20 == 0 {
                    self.control1 = (self.control1 & !0x10) | (value & 0x10);
                }
                if old & 0x10 != 0 {
                    if value & 8 == 0 {
                        self.control1 = (self.control1 & !4) | (value & 4);
                    }
                    if value & 3 == 0 {
                        self.control1 &= !1;
                    }
                }
                self.control1 |= 0xaa;
            }
            0xffb2 if origin.is_mov() => {
                let old = self.control2;
                let missed_clear =
                    old & 0x20 != 0 && origin == WriteOrigin::MovByteAbs8 { pc_bit1: false };
                if value & 0x80 == 0 && self.seen_overflow {
                    self.control2 &= !0x80;
                    self.seen_overflow = false;
                }
                for (inhibit, bit) in [(0x40, 0x20), (0x10, 8)] {
                    if value & inhibit == 0 && !(missed_clear && old & bit != 0 && value & bit == 0)
                    {
                        self.control2 = (self.control2 & !bit) | (value & bit);
                    }
                }
                self.control2 |= 0x57;
            }
            0xffb3 if self.control1 & 0x40 != 0 => self.count = value,
            _ => {}
        }
        self.last = self.tick(now, clocks);
        Ok(())
    }
    pub fn set_power(
        &mut self,
        gate: bool,
        source_available: bool,
        reset_held: bool,
        now: Time,
        clocks: &Clocks,
    ) {
        self.gate = gate;
        self.source_available = source_available;
        self.reset_held = reset_held;
        self.rosc_last = clocks.ticks(now, Tap::on_chip(1));
        self.last = self.tick(now, clocks);
    }
    pub fn reset(&mut self, watchdog: bool, now: Time, clocks: &Clocks) {
        *self = Self::default();
        if watchdog {
            self.control1 |= 1;
        }
        self.rosc_last = clocks.ticks(now, Tap::on_chip(1));
    }
    pub fn interrupt(&self) -> bool {
        self.interrupt_with_enable(0)
    }
    pub(crate) fn interrupt_with_enable(&self, retained: u8) -> bool {
        !self.reset_held && self.control2 & 0xa0 == 0xa0 && (self.control2 | retained) & 8 != 0
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.running() || self.control2 & 0xa0 == 0xa0 {
            return Ok(None);
        }
        let Some(tap) = self.tap() else {
            return Ok(None);
        };
        let count = 256 - u64::from(self.count);
        let at = if self.mode & 15 <= 3 {
            clocks.edge(
                self.rosc_last + count * 2048 - u64::from(self.rosc_phase),
                Tap::on_chip(1),
            )?
        } else {
            clocks.edge(self.last + count, tap)?
        };
        Ok(Some(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qualified_writes_use_the_old_latch_and_erratum_does_not_discard_ovf_clear() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut w = Watchdog::default();
        for (value, expected) in [(0x9e, 0xbe), (0xa2, 0xba), (0x8e, 0xaa)] {
            w.write(0xffb1, value, WriteOrigin::MovByte, Time::ZERO, &c)
                .unwrap();
            assert_eq!(w.peek(0xffb1), expected);
        }
        w.write(0xffb2, 0x28, WriteOrigin::MovByte, Time::ZERO, &c)
            .unwrap();
        assert_eq!(w.peek(0xffb2), 0x7f);
        w.control2 |= 0x80;
        w.read(0xffb2);
        w.write(
            0xffb2,
            0x47,
            WriteOrigin::MovByteAbs8 { pc_bit1: false },
            Time::ZERO,
            &c,
        )
        .unwrap();
        assert_eq!(w.peek(0xffb2), 0x7f);
        w.write(
            0xffb2,
            0x87,
            WriteOrigin::MovByteAbs8 { pc_bit1: true },
            Time::ZERO,
            &c,
        )
        .unwrap();
        assert_eq!(w.peek(0xffb2), 0x57);
    }
    #[test]
    fn private_divider_resumes_its_remaining_cycles_after_reset_release() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut w = Watchdog::default();
        let at = c.edge(713, Tap::on_chip(1)).unwrap();
        w.reset(true, at, &c);
        w.set_power(true, true, true, at, &c);
        let release = c.edge(1225, Tap::on_chip(1)).unwrap();
        assert!(!w.sync(release, &c));
        assert_eq!(w.count, 0);
        w.set_power(true, true, false, release, &c);
        w.write(0xffb1, 0x5e, WriteOrigin::MovByte, release, &c)
            .unwrap();
        w.write(0xffb3, 255, WriteOrigin::Other, release, &c)
            .unwrap();
        let due = c.edge(3273, Tap::on_chip(1)).unwrap();
        assert_eq!(w.deadline(&c).unwrap(), Some(due));
        assert!(w.sync(due, &c));
    }
}
