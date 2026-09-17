//! Shared clock phases. A peripheral records a consumed divider-edge ordinal;
//! stopping a downstream gate does not restart the oscillator or its divider.
use crate::{
    error::Error,
    time::{Clock, Time, TimeError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    System,
    Watch,
    OnChip,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tap {
    pub source: Source,
    pub divide: u32,
}
impl Tap {
    pub const fn system(divide: u32) -> Self {
        Self {
            source: Source::System,
            divide,
        }
    }
    pub const fn watch(divide: u32) -> Self {
        Self {
            source: Source::Watch,
            divide,
        }
    }
    pub const fn on_chip(divide: u32) -> Self {
        Self {
            source: Source::OnChip,
            divide,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frequencies {
    pub main_hz: u64,
    pub watch_hz: u64,
    pub on_chip_hz: u64,
}
impl Default for Frequencies {
    fn default() -> Self {
        Self {
            main_hz: 3_686_400,
            watch_hz: 32_768,
            on_chip_hz: 1_310_720,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clocks {
    pub frequencies: Frequencies,
    system: Clock,
    watch: Clock,
    on_chip: Clock,
    system_numerator: u64,
    system_denominator: u64,
    revision: u64,
}
impl Clocks {
    pub fn new(now: Time, frequencies: Frequencies) -> Result<Self, Error> {
        Ok(Self {
            frequencies,
            system: Clock::new(now, frequencies.main_hz, 1)?,
            watch: Clock::new(now, frequencies.watch_hz, 1)?,
            on_chip: Clock::new(now, frequencies.on_chip_hz, 1)?,
            system_numerator: frequencies.main_hz,
            system_denominator: 1,
            revision: 0,
        })
    }
    fn source(&self, source: Source) -> &Clock {
        match source {
            Source::System => &self.system,
            Source::Watch => &self.watch,
            Source::OnChip => &self.on_chip,
        }
    }
    /// Edges at `now` have occurred. Owners settle same-time conflicts before
    /// issuing a CPU access; external run horizons remain exclusive.
    pub fn ticks(&self, now: Time, tap: Tap) -> u64 {
        let c = self.source(tap.source);
        let inclusive = Time::from_raw(now.raw().saturating_add(1));
        let elapsed = c.edges_before(inclusive);
        (c.ordinal().saturating_add(elapsed)) / u64::from(tap.divide)
    }
    pub fn edge(&self, tick: u64, tap: Tap) -> Result<Time, Error> {
        let target = tick
            .checked_mul(u64::from(tap.divide))
            .ok_or(TimeError::Overflow)?;
        let c = self.source(tap.source);
        let delta = target.checked_sub(c.ordinal()).ok_or(TimeError::Reversed)?;
        Ok(c.after(delta)?)
    }
    pub fn after(&self, now: Time, edges: u64, tap: Tap) -> Result<Time, Error> {
        let target = self
            .ticks(now, tap)
            .checked_add(edges)
            .ok_or(TimeError::Overflow)?;
        self.edge(target, tap)
    }
    /// Called only at an established clock-switch boundary, after all system-
    /// clock users have synchronized under the previous frequency.
    pub fn set_system(&mut self, now: Time, numerator: u64, denominator: u64) -> Result<(), Error> {
        if (numerator, denominator) == (self.system_numerator, self.system_denominator) {
            return Ok(());
        }
        let ordinal = self.ticks(now, Tap::system(1));
        let mut c = Clock::new(now, numerator, denominator)?;
        c.ordinal = ordinal;
        let revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        self.system = c;
        self.revision = revision;
        self.system_numerator = numerator;
        self.system_denominator = denominator;
        Ok(())
    }
    pub fn system_rate(&self) -> (u64, u64) {
        (self.system_numerator, self.system_denominator)
    }
}

/// An obligation to consume source/divider edges, not a fixed wall-time delay.
/// The timestamp is a disposable cache for the current clock revision. A clock
/// switch changes the projection, not the outstanding edge target. Downstream
/// gating retains the number of unconsumed edges and rejoins the shared divider
/// phase on resume. It does not replay the old wall-time remainder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockWait {
    tap: Tap,
    state: WaitState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitState {
    Running {
        target: u64,
        cached: Time,
        revision: u64,
    },
    Paused {
        remaining: u64,
    },
    Ready(Time),
}
impl ClockWait {
    pub fn after(now: Time, edges: u64, tap: Tap, clocks: &Clocks) -> Result<Self, Error> {
        if tap.divide == 0 {
            return Err(Error::BadInput("zero clock divider"));
        }
        let state = if edges == 0 {
            WaitState::Ready(now)
        } else {
            let target = clocks
                .ticks(now, tap)
                .checked_add(edges)
                .ok_or(TimeError::Overflow)?;
            WaitState::Running {
                target,
                cached: clocks.edge(target, tap)?,
                revision: clocks.revision,
            }
        };
        Ok(Self { tap, state })
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        match self.state {
            WaitState::Running {
                target,
                cached,
                revision,
            } => Ok(Some(if revision == clocks.revision {
                cached
            } else {
                clocks.edge(target, self.tap)?
            })),
            WaitState::Paused { .. } => Ok(None),
            WaitState::Ready(at) => Ok(Some(at)),
        }
    }
    pub fn pause(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.deadline(clocks)?.is_some_and(|at| at < now) {
            return Err(TimeError::Reversed.into());
        }
        let remaining = match self.state {
            WaitState::Running { target, .. } => target
                .checked_sub(clocks.ticks(now, self.tap))
                .ok_or(TimeError::Reversed)?,
            WaitState::Ready(at) if at >= now => 0,
            WaitState::Ready(_) => return Err(TimeError::Reversed.into()),
            WaitState::Paused { .. } => return Ok(()),
        };
        self.state = WaitState::Paused { remaining };
        Ok(())
    }
    pub fn resume(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if let WaitState::Paused { remaining } = self.state {
            *self = Self::after(now, remaining, self.tap, clocks)?;
        }
        Ok(())
    }
    /// Select another prescaler output without discarding unfinished work.
    /// Already consumed edges belong to the old source; the remaining count
    /// rejoins the selected divider's existing phase. A closed gate stays closed.
    pub fn select(&mut self, now: Time, tap: Tap, clocks: &Clocks) -> Result<(), Error> {
        if tap.divide == 0 {
            return Err(Error::BadInput("zero clock divider"));
        }
        if self.tap != tap {
            let running = !matches!(self.state, WaitState::Paused { .. });
            self.pause(now, clocks)?;
            self.tap = tap;
            if running {
                self.resume(now, clocks)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn divided_clocks_share_their_source_phase() {
        let c = Clocks::new(Time::ZERO, Frequencies::default()).unwrap();
        let t = c.edge(17, Tap::system(4)).unwrap();
        assert_eq!(c.ticks(t, Tap::system(1)), 68);
        assert_eq!(c.ticks(t, Tap::system(4)), 17);
        assert_eq!(c.ticks(Time::from_raw(t.raw() - 1), Tap::system(4)), 16);
    }
    #[test]
    fn clock_change_preserves_edge_ordinal() {
        let mut c = Clocks::new(Time::ZERO, Frequencies::default()).unwrap();
        let t = c.edge(100, Tap::system(1)).unwrap();
        c.set_system(t, 32768, 1).unwrap();
        assert_eq!(c.ticks(t, Tap::system(1)), 100);
        assert_eq!(
            c.ticks(c.after(t, 4, Tap::system(1)).unwrap(), Tap::system(4)),
            26
        );
    }
}
