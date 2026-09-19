//! RTC and its alternate free-running counter. Busy entry and exit occur
//! separately from the second rollover. The busy interval lasts 62.5 ms at
//! the selected phase in docs/accuracy/rtc.md.
use super::clocks::{Clocks, Tap};
use crate::{error::Error, time::Time};
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Rtc {
    pub flags: u8,
    data: [u8; 4],
    control1: u8,
    control2: u8,
    source: u8,
    phase: u16,
    last: u64,
    enabled: bool,
    pending: Option<CalendarUpdate>,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct CalendarUpdate {
    data: [u8; 4],
    pm: u8,
    flags: u8,
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
            pending: None,
        }
    }
}
impl Rtc {
    pub fn reset(&mut self, now: Time, clocks: &Clocks) {
        // RES/WDT reset RTCCSR alone. Time, controls, flags, and RTC divider
        // state survive; software RST has its own different reset domain.
        self.source = 8;
        self.last = clocks.ticks(now, self.tap());
    }
    pub fn uses_watch(&self) -> bool {
        self.tap().source == super::clocks::Source::Watch
    }
    pub fn output_tap(&self) -> Tap {
        if self.source & 0x10 != 0 {
            Tap::watch(1)
        } else {
            Tap::system(4 << ((self.source >> 5) & 3))
        }
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
        if self.source & 8 == 0 {
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
        if self.source & 8 == 0 {
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
                let pending = self.next_calendar();
                if self.control1 & 8 == 0 {
                    self.flags |= pending.flags & self.control2;
                }
                self.pending = Some(pending);
            }
            if self.phase.is_multiple_of(2048) {
                self.flags |= self.control2 & 1;
                if self.phase.is_multiple_of(4096) {
                    self.flags |= self.control2 & 2;
                }
            }
            if self.phase == 8192 {
                self.phase = 0;
                if let Some(pending) = self.pending.take() {
                    self.data = pending.data;
                    self.control1 = (self.control1 & !0x20) | pending.pm;
                    if self.control1 & 8 != 0 {
                        self.flags |= pending.flags & self.control2;
                    }
                }
            }
        }
        Ok(())
    }
    fn next_calendar(&self) -> CalendarUpdate {
        let mut update = CalendarUpdate {
            data: self.data,
            pm: self.control1 & 0x20,
            flags: 4,
        };
        // Each digit has a terminal comparator and its physical field width.
        // Values outside normal BCD follow those counters, not normalization.
        for i in 0..2 {
            let value = update.data[i] & 0x7f;
            let units = value & 15;
            let tens = value >> 4;
            if units != 9 {
                update.data[i] = (tens << 4) | ((units + 1) & 15);
                return update;
            }
            if tens != 5 {
                update.data[i] = ((tens + 1) & 7) << 4;
                return update;
            }
            update.data[i] = 0;
            update.flags |= 8 << i;
        }
        let hours = update.data[2] & 0x3f;
        let full_day = self.control1 & 0x40 != 0;
        if hours != if full_day { 0x23 } else { 0x11 } {
            update.data[2] = if hours & 15 == 9 {
                hours.wrapping_add(7) & 0x3f
            } else {
                (hours & 0x30) | ((hours + 1) & 15)
            };
            return update;
        }
        update.data[2] = 0;
        if !full_day {
            update.pm ^= 0x20;
            if update.pm != 0 {
                return update;
            }
        }
        update.flags |= 0x20;
        let day = update.data[3] & 7;
        update.data[3] = if day == 6 { 0 } else { (day + 1) & 7 };
        if update.data[3] == 0 {
            update.flags |= 0x40;
        }
        update
    }
    pub fn set_gate(&mut self, enabled: bool, now: Time, clocks: &Clocks) {
        self.enabled = enabled;
        self.last = clocks.ticks(now, self.tap());
    }
    pub fn read(&self, address: u16) -> u8 {
        match address {
            0xf067 => self.flags,
            0xf068..=0xf06b => {
                let value = self.data[usize::from(address - 0xf068)];
                if self.source & 8 != 0 {
                    (value & 0x7f) | if self.pending.is_some() { 0x80 } else { 0 }
                } else {
                    value
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
        self.sync(now, clocks)?;
        match address {
            0xf067 => self.flags &= value,
            0xf068..=0xf06b => {
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
                    self.pending = None;
                } else {
                    self.control1 = value & 0xf8;
                }
            }
            0xf06d => self.control2 = value,
            // The application-note decoder is 1xxx: calendar source.
            0xf06f => self.source = value & 0x7f,
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
        if !self.running() || !clocks.available(self.tap()) {
            return Ok(None);
        }
        Ok(Some(clocks.edge(self.last + self.distance(), self.tap())?))
    }
    pub fn interrupt(&self) -> Option<u8> {
        self.interrupt_with_enable(0)
    }
    pub(crate) fn interrupt_with_enable(&self, retained: u8) -> Option<u8> {
        let p = self.flags & (self.control2 | retained);
        (p != 0).then(|| 23 + p.trailing_zeros() as u8)
    }
}

impl Rtc {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        crate::state::require(
            self.phase < 8192
                && self.data[1] & !0x7f == 0
                && self.data[2] & !0x3f == 0
                && self.data[3] < 8
                && self.pending.is_none_or(|p| {
                    p.data[0] < 128
                        && p.data[1] < 128
                        && p.data[2] < 64
                        && p.data[3] < 8
                        && p.pm & !0x20 == 0
                        && p.flags & !0x7c == 0
                })
                && self.control1 & 7 == 0
                && self.source & !0x7f == 0
                && self.last.checked_add(8192).is_some(),
            "invalid RTC progress",
        )
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
    #[test]
    fn busy_update_retains_its_values_across_writes_and_stop() {
        let (mut r, c) = running();
        for (i, v) in [0x59, 0x59, 0x23, 6].into_iter().enumerate() {
            r.write(0xf068 + i as u16, v, Time::from_micros(900_000), &c)
                .unwrap();
        }
        r.sync(Time::from_micros(937_500), &c).unwrap();
        r.write(0xf068, 0x12, Time::from_micros(950_000), &c)
            .unwrap();
        assert_eq!(r.read(0xf068), 0x92);
        r.write(0xf06c, 0x48, Time::from_micros(960_000), &c)
            .unwrap();
        r.sync(Time::from_micros(2_000_000), &c).unwrap();
        assert_eq!(r.read(0xf068), 0x92);
        r.write(0xf06c, 0xc8, Time::from_micros(2_000_000), &c)
            .unwrap();
        r.sync(r.deadline(&c).unwrap().unwrap(), &c).unwrap();
        for a in 0xf068..=0xf06b {
            assert_eq!(r.read(a), 0);
        }
        assert_eq!(r.read(0xf067), 0x7f);
    }
    #[test]
    fn raw_digits_follow_field_widths_and_terminal_comparators() {
        for (before, after) in [
            ([0x1a, 0, 0, 0], [0x1b, 0, 0, 0]),
            ([0x1f, 0, 0, 0], [0x10, 0, 0, 0]),
            ([0x69, 0, 0, 0], [0x70, 0, 0, 0]),
            ([0x59, 0x59, 0x2f, 7], [0, 0, 0x20, 7]),
            ([0x59, 0x59, 0x23, 7], [0, 0, 0, 0]),
        ] {
            let (mut r, c) = running();
            for (i, v) in before.into_iter().enumerate() {
                r.write(0xf068 + i as u16, v, Time::ZERO, &c).unwrap();
            }
            r.sync(Time::from_micros(1_000_000), &c).unwrap();
            for (i, v) in after.into_iter().enumerate() {
                assert_eq!(r.read(0xf068 + i as u16), v);
            }
        }
    }
    #[test]
    fn calendar_aliases_do_not_leak_the_binary_counter_high_bit_into_busy() {
        for source in 8..16 {
            let (mut r, c) = running();
            r.write(0xf06f, source, Time::ZERO, &c).unwrap();
            r.sync(Time::from_micros(1_000_000), &c).unwrap();
            assert_eq!(r.read(0xf068), 1);
            assert_eq!(r.read(0xf06f), source);
        }
        let c = Clocks::new(
            Time::ZERO,
            super::super::clocks::Frequencies {
                main_hz: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        let mut r = Rtc::default();
        r.write(0xf06f, 0, Time::ZERO, &c).unwrap();
        r.write(0xf06c, 0x80, Time::ZERO, &c).unwrap();
        r.sync(Time::from_micros(1200), &c).unwrap();
        assert_eq!(r.read(0xf068), 0x96);
        r.write(0xf06f, 0xf, Time::from_micros(1200), &c).unwrap();
        assert_eq!(r.read(0xf068), 0x16);
    }
}
