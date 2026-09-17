//! Shared clock phases. A peripheral records a consumed divider-edge ordinal;
//! stopping a downstream gate does not restart the oscillator or its divider.
mod prescaler;
use crate::{
    error::Error,
    time::{Clock, Time, TimeError},
};
use prescaler::Prescaler;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    System,
    Cpu,
    Watch,
    OnChip,
    Oscillator,
    Subclock,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tap {
    pub source: Source,
    pub divide: u32,
}
impl Tap {
    pub const fn cpu() -> Self {
        Self {
            source: Source::Cpu,
            divide: 1,
        }
    }
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
    pub const fn oscillator() -> Self {
        Self {
            source: Source::Oscillator,
            divide: 1,
        }
    }
    pub const fn subclock() -> Self {
        Self {
            source: Source::Subclock,
            divide: 1,
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
    cpu: Clock,
    cpu_uses_subclock: bool,
    prescaler_s: Prescaler<13>,
    prescaler_w: Prescaler<8>,
    watch: Clock,
    on_chip: Clock,
    oscillator: Clock,
    subclock: Clock,
    system_numerator: u64,
    system_denominator: u64,
    revision: u64,
}
impl Clocks {
    pub fn new(now: Time, frequencies: Frequencies) -> Result<Self, Error> {
        Ok(Self {
            frequencies,
            system: Clock::new(now, frequencies.main_hz, 1)?,
            cpu: Clock::new(now, frequencies.main_hz, 1)?,
            cpu_uses_subclock: false,
            prescaler_s: Prescaler::new(),
            prescaler_w: Prescaler::new(),
            watch: Clock::new(now, frequencies.watch_hz, 1)?,
            on_chip: Clock::new(now, frequencies.on_chip_hz, 1)?,
            oscillator: Clock::new(now, frequencies.main_hz, 1)?,
            subclock: Clock::new(now, frequencies.watch_hz, 8)?,
            system_numerator: frequencies.main_hz,
            system_denominator: 1,
            revision: 0,
        })
    }
    fn source(&self, source: Source) -> &Clock {
        match source {
            Source::System => &self.system,
            Source::Cpu => &self.cpu,
            Source::Watch => &self.watch,
            Source::OnChip => &self.on_chip,
            Source::Oscillator => &self.oscillator,
            Source::Subclock => &self.subclock,
        }
    }
    /// Edges at `now` have occurred. Owners settle same-time conflicts before
    /// issuing a CPU access; external run horizons remain exclusive.
    pub fn ticks(&self, now: Time, tap: Tap) -> u64 {
        let raw = self.raw_ticks(now, tap.source);
        if tap.divide.is_power_of_two() {
            match (tap.source, tap.divide) {
                (Source::System, 2..=8192) => {
                    return self.prescaler_s.ticks(raw, tap.divide.trailing_zeros())
                }
                (Source::Watch, 8..=1024) => {
                    return self
                        .prescaler_w
                        .ticks(raw / 4, tap.divide.trailing_zeros() - 2)
                }
                _ => {}
            }
        }
        raw / u64::from(tap.divide)
    }
    fn raw_ticks(&self, now: Time, source: Source) -> u64 {
        let c = self.source(source);
        let inclusive = Time::from_raw(now.raw().saturating_add(1));
        c.ordinal().saturating_add(c.edges_before(inclusive))
    }
    pub fn edge(&self, tick: u64, tap: Tap) -> Result<Time, Error> {
        let target = match (tap.source, tap.divide) {
            (Source::System, 2..=8192) if tap.divide.is_power_of_two() => self
                .prescaler_s
                .parent_edge(tick, tap.divide.trailing_zeros())?,
            (Source::Watch, 8..=1024) if tap.divide.is_power_of_two() => self
                .prescaler_w
                .parent_edge(tick, tap.divide.trailing_zeros() - 2)?
                .checked_mul(4)
                .ok_or(TimeError::Overflow)?,
            _ => tick
                .checked_mul(u64::from(tap.divide))
                .ok_or(TimeError::Overflow)?,
        };
        let c = self.source(tap.source);
        let delta = target.checked_sub(c.ordinal()).ok_or(TimeError::Reversed)?;
        Ok(c.after(delta)?)
    }
    /// Divider outputs use a high first half-cycle; source muxes and downstream
    /// gates observe this physical phase, not parity of lifetime edge counts.
    pub fn high(&self, now: Time, tap: Tap) -> bool {
        let raw = self.raw_ticks(now, tap.source);
        match (tap.source, tap.divide) {
            (Source::System, 2..=8192) if tap.divide.is_power_of_two() => {
                self.prescaler_s.high(raw, tap.divide.trailing_zeros())
            }
            (Source::Watch, 8..=1024) if tap.divide.is_power_of_two() => self
                .prescaler_w
                .high(raw / 4, tap.divide.trailing_zeros() - 2),
            (_, 2..) => raw % u64::from(tap.divide) < u64::from(tap.divide / 2),
            // At a reference-clock rising edge its level is high. Between
            // edges the rational period places the falling edge halfway.
            _ => {
                let c = self.source(tap.source);
                let edge = c.after(raw - c.ordinal()).unwrap_or(now);
                let next = c.after(raw - c.ordinal() + 1).unwrap_or(Time::MAX);
                now.raw() - edge.raw() < (next.raw() - edge.raw()) / 2
            }
        }
    }
    pub(crate) fn set_prescalers(
        &mut self,
        now: Time,
        system: bool,
        watch: bool,
    ) -> Result<(), Error> {
        let s = self.raw_ticks(now, Source::System);
        let w = self.raw_ticks(now, Source::Watch) / 4;
        let changed_s = self.prescaler_s.set_running(system, s, true);
        let changed_w = self.prescaler_w.set_running(watch, w, false);
        if changed_s || changed_w {
            self.revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        }
        Ok(())
    }
    pub(crate) fn reset_prescalers(&mut self, now: Time) -> Result<(), Error> {
        self.prescaler_s.reset(self.raw_ticks(now, Source::System));
        self.prescaler_w
            .reset(self.raw_ticks(now, Source::Watch) / 4);
        self.revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        Ok(())
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
    pub fn select_system(&mut self, now: Time, source: Source, divide: u64) -> Result<(), Error> {
        let numerator = match source {
            Source::Oscillator => self.frequencies.main_hz,
            Source::OnChip => self.frequencies.on_chip_hz,
            Source::Watch => self.frequencies.watch_hz,
            _ => return Err(Error::BadInput("system clock needs an oscillator source")),
        };
        let ordinal = self.ticks(now, Tap::system(1));
        let c = Self::divided(self.source(source), now, divide, ordinal)?;
        let revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        self.system = c;
        self.revision = revision;
        self.system_numerator = numerator;
        self.system_denominator = divide;
        if !self.cpu_uses_subclock {
            self.select_cpu(now, false)?;
        }
        Ok(())
    }
    pub fn select_subclock(&mut self, now: Time, divide: u64) -> Result<(), Error> {
        self.subclock = Self::divided(&self.watch, now, divide, self.ticks(now, Tap::subclock()))?;
        if self.cpu_uses_subclock {
            self.select_cpu(now, true)?;
        }
        self.revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        Ok(())
    }
    pub fn select_cpu(&mut self, now: Time, subclock: bool) -> Result<(), Error> {
        self.cpu = Self::divided(
            if subclock {
                &self.subclock
            } else {
                &self.system
            },
            now,
            1,
            self.ticks(now, Tap::cpu()),
        )?;
        self.cpu_uses_subclock = subclock;
        self.revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        Ok(())
    }
    fn divided(source: &Clock, now: Time, divide: u64, ordinal: u64) -> Result<Clock, Error> {
        if divide == 0 {
            return Err(TimeError::ZeroFrequency.into());
        }
        let mut clock = *source;
        let edges = source.edges_before(Time::from_raw(now.raw().saturating_add(1)));
        clock.advance(edges / divide * divide)?;
        let remainder = u128::from(clock.remainder) * u128::from(divide);
        clock.whole = clock
            .whole
            .checked_mul(u128::from(divide))
            .and_then(|v| v.checked_add(remainder / u128::from(clock.denominator)))
            .ok_or(TimeError::Overflow)?;
        clock.remainder = (remainder % u128::from(clock.denominator)) as u64;
        clock.ordinal = ordinal;
        Ok(clock)
    }
    pub fn system_rate(&self) -> (u64, u64) {
        (self.system_numerator, self.system_denominator)
    }
    /// Starting the stopped oscillator establishes its new physical phase.
    /// STS waits use this undivided source, independently of the CPU divider.
    pub fn restart_oscillator(&mut self, now: Time) -> Result<(), Error> {
        let ordinal = self.ticks(now, Tap::oscillator());
        self.oscillator = Clock::new(now, self.frequencies.main_hz, 1)?;
        self.oscillator.ordinal = ordinal;
        self.revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        Ok(())
    }
    pub fn restart_on_chip(&mut self, now: Time) -> Result<(), Error> {
        let ordinal = self.ticks(now, Tap::on_chip(1));
        self.on_chip = Clock::new(now, self.frequencies.on_chip_hz, 1)?;
        self.on_chip.ordinal = ordinal;
        self.revision = self.revision.checked_add(1).ok_or(TimeError::Overflow)?;
        Ok(())
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
    fn shared_dividers_reset_and_hold_without_rephasing_watch_input() {
        let mut c = Clocks::new(
            Time::ZERO,
            Frequencies {
                main_hz: 1_000_000,
                watch_hz: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        let t = Time::from_micros;
        assert_eq!(c.ticks(t(6), Tap::system(4)), 1);
        c.reset_prescalers(t(6)).unwrap();
        c.set_prescalers(t(6), false, false).unwrap();
        assert_eq!(c.ticks(t(11), Tap::system(4)), 1);
        assert_eq!(c.ticks(t(11), Tap::watch(4)), 2);
        assert_eq!(c.ticks(t(11), Tap::watch(16)), 0);
        c.set_prescalers(t(11), true, true).unwrap();
        assert_eq!(c.edge(2, Tap::system(4)).unwrap(), t(15));
        // W starts with the next upstream phiW/4 edge at 12, then 16,20,24.
        assert_eq!(c.edge(1, Tap::watch(16)).unwrap(), t(24));
        assert!(c.high(t(12), Tap::system(4)));
        assert!(!c.high(t(13), Tap::system(4)));
        assert!(c.high(t(15), Tap::system(4)));
        c.set_prescalers(t(29), false, false).unwrap();
        assert_eq!(c.ticks(t(43), Tap::watch(16)), 1);
        c.set_prescalers(t(43), true, true).unwrap();
        // S restarts from zero; W resumes its retained five-input-edge phase.
        assert_eq!(c.after(t(43), 1, Tap::system(4)).unwrap(), t(47));
        assert_eq!(c.after(t(43), 1, Tap::watch(16)).unwrap(), t(52));
    }
    #[test]
    fn cpu_subclock_selection_does_not_replace_system_phi() {
        let mut c = Clocks::new(
            Time::ZERO,
            Frequencies {
                main_hz: 1_000_000,
                watch_hz: 100_000,
                ..Default::default()
            },
        )
        .unwrap();
        c.select_subclock(Time::ZERO, 2).unwrap();
        let now = Time::from_micros(37);
        c.select_cpu(now, true).unwrap();
        assert_eq!(c.after(now, 1, Tap::cpu()).unwrap(), Time::from_micros(40));
        assert_eq!(
            c.after(now, 1, Tap::system(1)).unwrap(),
            Time::from_micros(38)
        );
        assert_eq!(c.system_rate(), (1_000_000, 1));
    }
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
        c.select_system(t, Source::Watch, 1).unwrap();
        assert_eq!(c.ticks(t, Tap::system(1)), 100);
        assert_eq!(
            c.ticks(c.after(t, 4, Tap::system(1)).unwrap(), Tap::system(4)),
            26
        );
    }
}
