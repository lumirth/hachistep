//! Timer B1: interval/reload operation, shared prescaler phase, and lazy reads.
use super::clocks::{Clocks, Tap};
use crate::{error::Error, time::Time};
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
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
        self.advance(n)
    }
    fn advance(&mut self, n: u64) -> bool {
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
    ) -> Result<bool, Error> {
        let mut overflow = self.sync(now, clocks);
        if address == 0xf0d0 {
            let was_running = self.enabled && self.mode & 0x40 != 0;
            let old = clocks.high(now, self.tap());
            self.mode = value | 0x38;
            // A live mux change can itself supply a rising counter edge.
            if was_running && self.mode & 0x40 != 0 && !old && clocks.high(now, self.tap()) {
                overflow |= self.advance(1);
            }
        } else {
            // TLB feeds both latches. Stopping before a write is programming
            // guidance, rather than a hardware write-protection mechanism.
            self.load = value;
            self.count = value;
        }
        self.last = clocks.ticks(now, self.tap());
        Ok(overflow)
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.enabled || self.mode & 0x40 == 0 || !clocks.available(self.tap()) {
            return Ok(None);
        }
        Ok(Some(clocks.edge(
            self.last + (256 - u64::from(self.count)),
            self.tap(),
        )?))
    }
}

impl TimerB1 {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        crate::state::require(
            self.mode & 0x38 == 0x38 && self.last.checked_add(256).is_some(),
            "invalid Timer B1 progress",
        )
    }
}
