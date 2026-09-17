//! The BRC reload counter and basic-clock phase. Idle clocks are advanced
//! arithmetically; only edges that can change a pin, shift register, or status
//! need an appointment in the machine scheduler.
use super::super::clocks::{Clocks, Tap};
use crate::{
    error::Error,
    time::{Time, TimeError},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Baud {
    tap: Tap,
    consumed: u64,
    remaining: u16,
    pub half: u64,
    pub high: bool,
    last: Time,
    running: bool,
    external: bool,
}
impl Default for Baud {
    fn default() -> Self {
        Self {
            tap: Tap::system(1),
            consumed: 0,
            remaining: 256,
            half: 0,
            high: true,
            last: Time::ZERO,
            running: false,
            external: false,
        }
    }
}
impl Baud {
    pub fn sync(&mut self, now: Time, clocks: &Clocks, reload: u16) -> Result<(), Error> {
        let tick = clocks.ticks(now, self.tap);
        let elapsed = tick.saturating_sub(self.consumed);
        if self.running && !self.external {
            if elapsed >= u64::from(self.remaining) {
                let remainder = elapsed - u64::from(self.remaining);
                let halves = 1 + remainder / u64::from(reload);
                let after = remainder % u64::from(reload);
                self.half = self.half.checked_add(halves).ok_or(TimeError::Overflow)?;
                self.high ^= halves & 1 != 0;
                self.remaining = reload - after as u16;
                self.last = clocks.edge(tick - after, self.tap)?;
            } else {
                self.remaining -= elapsed as u16;
            }
        }
        self.consumed = tick;
        Ok(())
    }
    /// Settle the old selection before calling this. Source selection and
    /// ordinary gating retain unfinished BRC work and divider polarity.
    pub fn select(&mut self, now: Time, clocks: &Clocks, tap: Tap, running: bool, external: bool) {
        self.tap = tap;
        self.consumed = clocks.ticks(now, tap);
        self.running = running;
        self.external = external;
    }
    pub fn reload(&mut self, count: u16) {
        self.remaining = count;
    }
    pub fn connect(&mut self, high: bool) {
        self.high = high;
    }
    pub fn external_edge(&mut self, now: Time, high: bool) -> Result<bool, Error> {
        if !self.running || !self.external || self.high == high {
            return Ok(false);
        }
        self.high = high;
        self.half = self.half.checked_add(1).ok_or(TimeError::Overflow)?;
        self.last = now;
        Ok(true)
    }
    pub fn next_fall(&self) -> u64 {
        self.half + if self.high { 1 } else { 2 }
    }
    pub fn boundary(&self, span: u64, now: Time) -> u64 {
        if self.half % span == 0 && self.last == now {
            self.half
        } else {
            self.half + span - self.half % span
        }
    }
    pub fn deadline(&self, half: u64, clocks: &Clocks, reload: u16) -> Result<Option<Time>, Error> {
        if !self.running || self.external {
            return Ok(None);
        }
        let delta = half.checked_sub(self.half).ok_or(TimeError::Reversed)?;
        if delta == 0 {
            return Ok(Some(self.last));
        }
        let count = (delta - 1)
            .checked_mul(u64::from(reload))
            .and_then(|n| n.checked_add(u64::from(self.remaining)))
            .and_then(|n| n.checked_add(self.consumed))
            .ok_or(TimeError::Overflow)?;
        Ok(Some(clocks.edge(count, self.tap)?))
    }
}
