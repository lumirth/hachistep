//! The BRC reload counter and basic-clock phase. Idle clocks are advanced
//! arithmetically; only edges that can change a pin, shift register, or status
//! need an appointment in the machine scheduler.
use super::super::clocks::{Clocks, Tap};
use crate::{
    error::Error,
    time::{Time, TimeError},
};

/// While the BRC counts, `consumed` is the tick its current half began at
/// (or its count was last loaded at) and `remaining` the ticks from there
/// to the next half, so synchronizing between halves changes nothing; while
/// it does not, `consumed` follows the clock and `remaining` is retained.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct Baud {
    tap: Tap,
    consumed: u64,
    remaining: u16,
    pub half: u64,
    pub high: bool,
    last: Time,
    running: bool,
    external: bool,
    #[borsh(skip)]
    next: NextHalf,
    /// `last` awaits the edge time of the half `step_to` reached.
    #[borsh(skip)]
    stale: bool,
}
/// The time of the next half's edge under one clock revision; not state.
#[derive(Clone, Debug, Default)]
struct NextHalf(core::cell::Cell<Option<(u64, u64, Time)>>);
impl PartialEq for NextHalf {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for NextHalf {}
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
            next: NextHalf::default(),
            stale: false,
        }
    }
}
impl Baud {
    fn counting(&self) -> bool {
        self.running && !self.external
    }
    /// Whether no half begins through `now`: the BRC then needs no
    /// synchronization.
    pub fn quiet_through(&self, now: Time, clocks: &Clocks) -> bool {
        if !self.counting() {
            return false;
        }
        let target = self.consumed + u64::from(self.remaining);
        let revision = clocks.revision();
        let at = match self.next.0.get() {
            Some((r, t, at)) if r == revision && t == target => at,
            _ => match clocks.edge(target, self.tap) {
                Ok(at) => {
                    self.next.0.set(Some((revision, target, at)));
                    at
                }
                Err(_) => return false,
            },
        };
        now < at
    }
    pub fn sync(&mut self, now: Time, clocks: &Clocks, reload: u16) -> Result<(), Error> {
        if self.quiet_through(now, clocks) {
            self.time(clocks)?;
            return Ok(());
        }
        self.stale = false;
        let tick = clocks.ticks(now, self.tap);
        if !self.counting() {
            self.consumed = tick;
            return Ok(());
        }
        let elapsed = tick.saturating_sub(self.consumed);
        if elapsed >= u64::from(self.remaining) {
            let remainder = elapsed - u64::from(self.remaining);
            let halves = 1 + remainder / u64::from(reload);
            let after = remainder % u64::from(reload);
            self.half = self.half.checked_add(halves).ok_or(TimeError::Overflow)?;
            self.high ^= halves & 1 != 0;
            self.remaining = reload;
            self.consumed = tick - after;
            self.last = clocks.edge(self.consumed, self.tap)?;
        }
        Ok(())
    }
    /// The counting form's `remaining` measured from `now`, as the BRC
    /// shows it; the caller has synchronized to `now`.
    fn settle(&mut self, now: Time, clocks: &Clocks) {
        let tick = clocks.ticks(now, self.tap);
        if self.counting() {
            let elapsed = tick.saturating_sub(self.consumed);
            self.remaining -= elapsed.min(u64::from(self.remaining) - 1) as u16;
        }
        self.consumed = tick;
    }
    /// Settle the old selection before calling this. Source selection and
    /// ordinary gating retain unfinished BRC work and divider polarity.
    pub fn select(&mut self, now: Time, clocks: &Clocks, tap: Tap, running: bool, external: bool) {
        if (self.tap, self.running, self.external) == (tap, running, external) {
            return;
        }
        self.settle(now, clocks);
        self.tap = tap;
        self.consumed = clocks.ticks(now, tap);
        self.running = running;
        self.external = external;
    }
    pub fn reload(&mut self, count: u16, now: Time, clocks: &Clocks) {
        self.consumed = clocks.ticks(now, self.tap);
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
    /// The half the BRC has reached by `now`, as `sync` would leave it.
    pub fn half_at(&self, now: Time, clocks: &Clocks, reload: u16) -> u64 {
        if !self.counting() {
            return self.half;
        }
        let elapsed = clocks.ticks(now, self.tap).saturating_sub(self.consumed);
        if elapsed < u64::from(self.remaining) {
            return self.half;
        }
        self.half + 1 + (elapsed - u64::from(self.remaining)) / u64::from(reload)
    }
    /// Count on to half `half`, which a counting BRC reaches; its edge time
    /// is found only when asked for (`time`).
    pub fn step_to(&mut self, half: u64, reload: u16) {
        let Some(halves) = half.checked_sub(self.half).filter(|n| *n != 0) else {
            return;
        };
        self.consumed += u64::from(self.remaining) + (halves - 1) * u64::from(reload);
        self.remaining = reload;
        self.high ^= halves & 1 != 0;
        self.half = half;
        self.stale = true;
    }
    /// The edge time of the current half after `step_to`.
    pub fn time(&mut self, clocks: &Clocks) -> Result<Time, Error> {
        if self.stale {
            self.last = clocks.edge(self.consumed, self.tap)?;
            self.stale = false;
        }
        Ok(self.last)
    }
    /// Whether the BRC counts a running clock, so `half_at` advances.
    pub fn advancing(&self, clocks: &Clocks) -> bool {
        self.counting() && clocks.available(self.tap)
    }
    pub fn next_fall(&self) -> u64 {
        self.half + if self.high { 1 } else { 2 }
    }
    pub fn boundary(&self, span: u64, now: Time) -> u64 {
        if self.half.is_multiple_of(span) && self.last == now {
            self.half
        } else {
            self.half + span - self.half % span
        }
    }
    pub fn deadline(&self, half: u64, clocks: &Clocks, reload: u16) -> Result<Option<Time>, Error> {
        if !self.running || self.external || !clocks.available(self.tap) {
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

impl Baud {
    pub(super) fn validate(&self, now: Time) -> Result<(), Error> {
        self.tap.validate()?;
        crate::state::require(
            [
                Tap::system(1),
                Tap::system(16),
                Tap::system(64),
                Tap::watch(1),
            ]
            .contains(&self.tap),
            "invalid baud clock",
        )?;
        crate::state::require(
            (1..=256).contains(&self.remaining)
                && self.last <= now
                && self.half.checked_add(1024).is_some()
                && self.consumed.checked_add(65536).is_some(),
            "invalid baud counter",
        )
    }
}
