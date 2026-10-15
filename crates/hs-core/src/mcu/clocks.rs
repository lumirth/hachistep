//! Shared clock phases. Each peripheral records how many divider edges it has
//! consumed. Clocks keep their shared phase across consumer stops and restarts.
mod cursor;
mod domain;
mod prescaler;
use crate::{
    error::Error,
    time::{Time, TimeError},
};
pub(crate) use cursor::{CpuCursor, CpuWindow};
use domain::Domain;
use prescaler::Prescaler;

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum Source {
    System = 0,
    Cpu = 1,
    Watch = 2,
    OnChip = 3,
    Oscillator = 4,
    Subclock = 5,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Clocks {
    pub frequencies: Frequencies,
    system: Domain,
    cpu: Domain,
    cpu_uses_subclock: bool,
    prescaler_s: Prescaler<13>,
    prescaler_w: Prescaler<8>,
    watch: Domain,
    on_chip: Domain,
    oscillator: Domain,
    subclock: Domain,
    watch_crystal: Domain,
    watch_uses_on_chip: bool,
    subclock_divide: u64,
    system_source: Source,
    system_divide: u64,
    system_numerator: u64,
    system_denominator: u64,
    #[borsh(skip)]
    revision: Revision,
    #[borsh(skip)]
    lock: LockMemo,
    #[borsh(skip)]
    projection: Projection,
}
/// The source ticks last projected for each source, with the span of times
/// that projects to the same count; not state. Owners synchronized at one
/// instant, and slow sources polled between their edges, reuse it.
#[derive(Clone, Debug, Default)]
struct Projection([core::cell::Cell<Span>; 6]);
#[derive(Clone, Copy, Debug, Default)]
struct Span {
    /// The clock revision the span was projected under: every domain is
    /// fixed within one.
    revision: u64,
    /// `ticks` is the count for every time in `from..until` (raw times).
    from: u128,
    until: u128,
    ticks: u64,
}
impl PartialEq for Projection {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Projection {}
/// A clock revision no projection has seen: revisions are unique across
/// every `Clocks`, so a projection cached under one is never mistaken for
/// another's. Zero is never issued; deserialized caches hold it.
pub(crate) fn fresh_revision() -> u64 {
    static NEXT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}
/// Identifies the clock configuration projections were cached under; not
/// state, so equal machines may hold different revisions.
#[derive(Clone, Copy, Debug)]
struct Revision(u64);
impl Default for Revision {
    fn default() -> Self {
        Self(fresh_revision())
    }
}
impl PartialEq for Revision {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Revision {}
/// `Clocks::cpu_lock` for one revision; not state.
#[derive(Clone, Debug, Default)]
struct LockMemo(core::cell::Cell<Option<(u64, Option<u64>)>>);
impl PartialEq for LockMemo {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for LockMemo {}
pub(crate) mod startup;
pub(crate) struct SourcePower {
    pub main: bool,
    pub oscillator: bool,
    pub watch: bool,
    pub crystal: bool,
    pub on_chip: bool,
    pub watch_on_chip: bool,
}
impl Clocks {
    pub fn new(now: Time, frequencies: Frequencies) -> Result<Self, Error> {
        Ok(Self {
            frequencies,
            system: Domain::new(now, frequencies.main_hz, 1)?,
            cpu: Domain::new(now, frequencies.main_hz, 1)?,
            cpu_uses_subclock: false,
            prescaler_s: Prescaler::new(),
            prescaler_w: Prescaler::new(),
            watch: Domain::new(now, frequencies.watch_hz, 1)?,
            on_chip: Domain::new(now, frequencies.on_chip_hz, 1)?,
            oscillator: Domain::new(now, frequencies.main_hz, 1)?,
            subclock: Domain::new(now, frequencies.watch_hz, 8)?,
            watch_crystal: Domain::new(now, frequencies.watch_hz, 1)?,
            watch_uses_on_chip: false,
            subclock_divide: 8,
            system_source: Source::Oscillator,
            system_divide: 1,
            system_numerator: frequencies.main_hz,
            system_denominator: 1,
            revision: Revision(fresh_revision()),
            lock: LockMemo::default(),
            projection: Projection::default(),
        })
    }
    fn source(&self, source: Source) -> &Domain {
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
        let memo = &self.projection.0[source as usize];
        let span = memo.get();
        if span.revision == self.revision.0 && (span.from..span.until).contains(&now.raw()) {
            return span.ticks;
        }
        let domain = self.source(source);
        let ticks = domain.ticks(now);
        // No exclusive cache endpoint exists beyond the terminal timestamp.
        if now == Time::MAX {
            return ticks;
        }
        // The count holds until the next source edge, after `now`.
        let until = if domain.running {
            let clock = &domain.clock;
            ticks
                .checked_sub(clock.ordinal())
                .and_then(|n| n.checked_add(1))
                .and_then(|n| clock.after(n).ok())
                .map_or(now.raw() + 1, |t| t.raw())
        } else {
            u128::MAX
        };
        memo.set(Span {
            revision: self.revision.0,
            from: now.raw(),
            until,
            ticks,
        });
        ticks
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
        let c = &self.source(tap.source).clock;
        let delta = target.checked_sub(c.ordinal()).ok_or(TimeError::Reversed)?;
        Ok(c.after(delta)?)
    }
    /// Next physical level change of a routed clock. Falling edges matter
    /// when a divider drives a board pin even if no counter uses that pin.
    pub fn next_transition(&self, now: Time, tap: Tap) -> Result<Option<Time>, Error> {
        if !self.available(tap) {
            return Ok(None);
        }
        let raw = self.raw_ticks(now, tap.source);
        let c = &self.source(tap.source).clock;
        if tap.divide == 1 {
            let edge = c.after(raw - c.ordinal())?;
            let next = c.after(raw - c.ordinal() + 1)?;
            let middle = Time::from_raw(edge.raw() + (next.raw() - edge.raw()) / 2);
            return Ok(Some(if middle > now { middle } else { next }));
        }
        let target = match (tap.source, tap.divide) {
            (Source::System, 2..=8192) if tap.divide.is_power_of_two() => self
                .prescaler_s
                .next_transition(raw, tap.divide.trailing_zeros())?,
            (Source::Watch, 8..=1024) if tap.divide.is_power_of_two() => self
                .prescaler_w
                .next_transition(raw / 4, tap.divide.trailing_zeros() - 2)?
                .checked_mul(4)
                .ok_or(TimeError::Overflow)?,
            _ => {
                let half = u64::from(tap.divide / 2);
                raw.checked_add(half - raw % half)
                    .ok_or(TimeError::Overflow)?
            }
        };
        Ok(Some(c.after(target - c.ordinal())?))
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
                let domain = self.source(tap.source);
                let now = domain.time(now);
                let c = &domain.clock;
                let edge = c.after(raw - c.ordinal()).unwrap_or(now);
                let next = c.after(raw - c.ordinal() + 1).unwrap_or(Time::MAX);
                now.raw() - edge.raw() < (next.raw() - edge.raw()) / 2
            }
        }
    }
    /// IIC2's SCL synchronization monitor includes half-phi obligations.
    pub(crate) fn system_half_ticks(&self, now: Time) -> Result<u64, Error> {
        self.ticks(now, Tap::system(1))
            .checked_mul(2)
            .and_then(|v| v.checked_add(u64::from(!self.high(now, Tap::system(1)))))
            .ok_or_else(|| TimeError::Overflow.into())
    }
    pub(crate) fn system_half_edge(&self, tick: u64) -> Result<Time, Error> {
        let a = self.edge(tick / 2, Tap::system(1))?;
        if tick & 1 == 0 {
            return Ok(a);
        }
        let b = self.edge(tick / 2 + 1, Tap::system(1))?;
        Ok(Time::from_raw(a.raw() + (b.raw() - a.raw()) / 2))
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
            self.revision = Revision(fresh_revision());
        }
        Ok(())
    }
    pub(crate) fn reset_prescalers(&mut self, now: Time) -> Result<(), Error> {
        self.prescaler_s.reset(self.raw_ticks(now, Source::System));
        self.prescaler_w
            .reset(self.raw_ticks(now, Source::Watch) / 4);
        self.revision = Revision(fresh_revision());
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
        let (numerator, denominator) = match source {
            Source::Oscillator => (self.frequencies.main_hz, 1),
            Source::OnChip => (self.frequencies.on_chip_hz, 1),
            Source::Watch if self.watch_uses_on_chip => (self.frequencies.on_chip_hz, 32),
            Source::Watch => (self.frequencies.watch_hz, 1),
            _ => return Err(Error::BadInput("system clock needs an oscillator source")),
        };
        let ordinal = self.ticks(now, Tap::system(1));
        let c = Self::divided(self.source(source), now, divide, ordinal)?;
        self.system = c;
        self.revision = Revision(fresh_revision());
        self.system_numerator = numerator;
        self.system_denominator = divide.checked_mul(denominator).ok_or(TimeError::Overflow)?;
        self.system_source = source;
        self.system_divide = divide;
        if !self.cpu_uses_subclock {
            self.select_cpu(now, false)?;
        }
        Ok(())
    }
    pub fn select_subclock(&mut self, now: Time, divide: u64) -> Result<(), Error> {
        self.subclock_divide = divide;
        self.subclock = Self::divided(&self.watch, now, divide, self.ticks(now, Tap::subclock()))?;
        if self.cpu_uses_subclock {
            self.select_cpu(now, true)?;
        }
        self.revision = Revision(fresh_revision());
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
        self.revision = Revision(fresh_revision());
        Ok(())
    }
    fn divided(source: &Domain, now: Time, divide: u64, ordinal: u64) -> Result<Domain, Error> {
        if divide == 0 {
            return Err(TimeError::ZeroFrequency.into());
        }
        let mut clock = source.clock;
        // A divided clock must still have a representable rational period,
        // including its remainder, not only a representable whole component.
        clock
            .whole
            .checked_mul(u128::from(clock.denominator))
            .and_then(|v| v.checked_add(u128::from(clock.remainder)))
            .and_then(|v| v.checked_mul(u128::from(divide)))
            .ok_or(TimeError::Overflow)?;
        let time = source.time(now);
        let edges = clock.edges_before(Time::from_raw(time.raw().saturating_add(1)));
        clock.advance(edges / divide * divide)?;
        let remainder = u128::from(clock.remainder) * u128::from(divide);
        clock.whole = clock
            .whole
            .checked_mul(u128::from(divide))
            .and_then(|v| v.checked_add(remainder / u128::from(clock.denominator)))
            .ok_or(TimeError::Overflow)?;
        clock.remainder = (remainder % u128::from(clock.denominator)) as u64;
        clock.ordinal = ordinal;
        Ok(Domain {
            clock,
            running: source.running,
            held_at: time,
        })
    }
    pub fn system_rate(&self) -> (u64, u64) {
        (self.system_numerator, self.system_denominator)
    }
    /// Starting the stopped oscillator establishes its new physical phase.
    /// STS waits use this undivided source, independently of the CPU divider.
    pub fn restart_oscillator(&mut self, now: Time) -> Result<(), Error> {
        self.oscillator.start(now, self.frequencies.main_hz)?;
        self.revision = Revision(fresh_revision());
        Ok(())
    }
    pub fn restart_on_chip(&mut self, now: Time) -> Result<(), Error> {
        self.on_chip.start(now, self.frequencies.on_chip_hz)?;
        self.revision = Revision(fresh_revision());
        Ok(())
    }
    /// Changes whenever a clock projection may have changed.
    pub(crate) fn revision(&self) -> u64 {
        self.revision.0
    }
    /// When every CPU clock edge is a system clock edge and the reverse, as
    /// while the CPU runs undivided on a running system clock: the CPU
    /// ordinal minus the system ordinal of the same edge (wrapping). Event
    /// times on either clock then convert to the other by addition.
    pub(crate) fn cpu_lock(&self) -> Option<u64> {
        if let Some((revision, lock)) = self.lock.0.get() {
            if revision == self.revision.0 {
                return lock;
            }
        }
        let lock = self.find_cpu_lock();
        self.lock.0.set(Some((self.revision.0, lock)));
        lock
    }
    fn find_cpu_lock(&self) -> Option<u64> {
        let (system, cpu) = (&self.system.clock, &self.cpu.clock);
        if self.cpu_uses_subclock
            || !self.system.running
            || !self.cpu.running
            || (system.whole, system.remainder, system.denominator)
                != (cpu.whole, cpu.remainder, cpu.denominator)
            || cpu.at < system.at
        {
            return None;
        }
        // The CPU clock is the system recurrence advanced to one of its
        // edges; check that it holds the same phase there.
        let edges = system.edges_before(Time::from_raw(cpu.at.raw().saturating_add(1)));
        let mut same = *system;
        same.advance(edges).ok()?;
        (same.at == cpu.at && same.fraction == cpu.fraction)
            .then(|| cpu.ordinal.wrapping_sub(same.ordinal))
    }
    /// The system clock ordinal of a system tap's `tick`-th edge.
    pub(crate) fn system_edge(&self, tick: u64, tap: Tap) -> Result<u64, Error> {
        debug_assert!(tap.source == Source::System);
        Ok(match tap.divide {
            2..=8192 if tap.divide.is_power_of_two() => self
                .prescaler_s
                .parent_edge(tick, tap.divide.trailing_zeros())?,
            _ => tick
                .checked_mul(u64::from(tap.divide))
                .ok_or(TimeError::Overflow)?,
        })
    }
    /// A system tap's ticks at the system clock's `ordinal`-th edge.
    pub(crate) fn system_ticks(&self, ordinal: u64, tap: Tap) -> u64 {
        debug_assert!(tap.source == Source::System);
        match tap.divide {
            2..=8192 if tap.divide.is_power_of_two() => {
                self.prescaler_s.ticks(ordinal, tap.divide.trailing_zeros())
            }
            _ => ordinal / u64::from(tap.divide),
        }
    }
    pub fn available(&self, tap: Tap) -> bool {
        self.source(tap.source).running
            && match (tap.source, tap.divide) {
                (Source::System, 2..=8192) => self.prescaler_s.running(),
                (Source::Watch, 8..=1024) => self.prescaler_w.running(),
                _ => true,
            }
    }
    /// Source consumers are fixed hardware: main phi, the watch mux, and WDT.
    /// Call after settling their old rules. Ordinary register writes that do
    /// not change availability leave every phase and projection untouched.
    pub(crate) fn power_sources(&mut self, now: Time, power: SourcePower) -> Result<(), Error> {
        let watch_changed =
            self.watch.running != power.watch || self.watch_uses_on_chip != power.watch_on_chip;
        let system_changed = self.system.running != power.main;
        if watch_changed {
            if self.cpu_uses_subclock {
                self.cpu.stop(now)?;
            }
            self.subclock.stop(now)?;
            self.watch.stop(now)?;
        }
        if system_changed {
            if !self.cpu_uses_subclock {
                self.cpu.stop(now)?;
            }
            self.system.stop(now)?;
        }
        let oscillator_changed =
            self.oscillator
                .set_running(power.oscillator, now, self.frequencies.main_hz)?;
        let rosc_changed =
            self.on_chip
                .set_running(power.on_chip, now, self.frequencies.on_chip_hz)?;
        let crystal_changed =
            self.watch_crystal
                .set_running(power.crystal, now, self.frequencies.watch_hz)?;
        if watch_changed {
            self.watch_uses_on_chip = power.watch_on_chip;
            if power.watch {
                let source = if power.watch_on_chip {
                    &self.on_chip
                } else {
                    &self.watch_crystal
                };
                self.watch = Self::divided(
                    source,
                    now,
                    if power.watch_on_chip { 32 } else { 1 },
                    self.watch.ticks(now),
                )?;
            }
            self.select_subclock(now, self.subclock_divide)?;
        }
        if system_changed {
            if power.main {
                self.select_system(now, self.system_source, self.system_divide)?;
            } else if !self.cpu_uses_subclock {
                self.select_cpu(now, false)?;
            }
        }
        if watch_changed || system_changed || oscillator_changed || rosc_changed || crystal_changed
        {
            self.revision = Revision(fresh_revision());
        }
        Ok(())
    }
}

/// An obligation to consume source/divider edges, not a fixed wall-time delay.
/// The timestamp is a disposable cache for the current clock revision. A clock
/// switch changes the projection, not the outstanding edge target. Downstream
/// gating retains the number of unconsumed edges and rejoins the shared divider
/// phase on resume. It does not replay the old wall-time remainder.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[borsh(init = invalidate)]
pub struct ClockWait {
    tap: Tap,
    state: WaitState,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum WaitState {
    Running {
        target: u64,
        #[borsh(skip)]
        cached: Time,
        #[borsh(skip)]
        revision: u64,
    } = 0,
    Paused {
        remaining: u64,
    } = 1,
    Ready(Time) = 2,
}
impl ClockWait {
    fn invalidate(&mut self) {
        if let WaitState::Running { revision, .. } = &mut self.state {
            *revision = u64::MAX;
        }
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.tap.validate()
    }
    pub(crate) fn uses(&self, tap: Tap) -> bool {
        self.tap == tap
    }

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
                cached: if clocks.available(tap) {
                    clocks.edge(target, tap)?
                } else {
                    Time::MAX
                },
                revision: clocks.revision.0,
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
            } if clocks.available(self.tap) => Ok(Some(if revision == clocks.revision.0 {
                cached
            } else {
                clocks.edge(target, self.tap)?
            })),
            WaitState::Running { .. } | WaitState::Paused { .. } => Ok(None),
            WaitState::Ready(at) => Ok(Some(at)),
        }
    }
    pub(crate) fn later_edge(&self, count: u64, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if count == 0 {
            return self.deadline(clocks);
        }
        match self.state {
            WaitState::Running { target, .. } if clocks.available(self.tap) => {
                Ok(Some(clocks.edge(
                    target.checked_add(count).ok_or(TimeError::Overflow)?,
                    self.tap,
                )?))
            }
            WaitState::Ready(at) => Self::after(at, count, self.tap, clocks)?.deadline(clocks),
            _ => Ok(None),
        }
    }
    /// The CPU clock ordinal of the `count`-th edge after the next, when
    /// it is on the CPU clock or on a system clock locked to it
    /// (`Clocks::cpu_lock`). None otherwise, and while not running.
    pub(crate) fn cpu_edge(&self, count: u64, clocks: &Clocks) -> Option<u64> {
        let (first, period) = self.cpu_run(clocks)?;
        first.checked_add(count.checked_mul(period)?)
    }
    /// First edge and period on the CPU clock while the sources are locked.
    /// A shared prescaler changes the first edge's phase, not its stride.
    #[inline]
    pub(crate) fn cpu_run(&self, clocks: &Clocks) -> Option<(u64, u64)> {
        let WaitState::Running { target, .. } = self.state else {
            return None;
        };
        if !clocks.available(self.tap) {
            return None;
        }
        match self.tap.source {
            Source::Cpu if self.tap.divide == 1 => Some((target, 1)),
            Source::System => Some((
                clocks
                    .system_edge(target, self.tap)
                    .ok()?
                    .wrapping_add(clocks.cpu_lock()?),
                u64::from(self.tap.divide),
            )),
            _ => None,
        }
    }
    /// `after` from the time of the CPU clock's `ordinal`-th edge, when
    /// that edge places the tap without a timestamp (see `cpu_edge`).
    pub(crate) fn after_cpu(
        ordinal: u64,
        edges: u64,
        tap: Tap,
        clocks: &Clocks,
    ) -> Option<Result<Self, Error>> {
        if edges == 0 || tap.divide == 0 || !clocks.available(tap) {
            return None;
        }
        let ticks = match tap.source {
            Source::Cpu if tap.divide == 1 => ordinal,
            Source::System => clocks.system_ticks(ordinal.wrapping_sub(clocks.cpu_lock()?), tap),
            _ => return None,
        };
        Some((|| {
            let target = ticks.checked_add(edges).ok_or(TimeError::Overflow)?;
            Ok(Self {
                tap,
                state: WaitState::Running {
                    target,
                    cached: clocks.edge(target, tap)?,
                    revision: clocks.revision.0,
                },
            })
        })())
    }
    /// Upcoming edges of a running wait strictly before `end`.
    pub(crate) fn edges_before(&self, end: Time, clocks: &Clocks) -> u64 {
        match self.state {
            WaitState::Running { target, .. } if clocks.available(self.tap) && end > Time::ZERO => {
                let done = clocks.ticks(Time::from_raw(end.raw() - 1), self.tap);
                (done + 1).saturating_sub(target)
            }
            _ => 0,
        }
    }
    /// Move a running wait `edges` edges on, as if each had been followed.
    pub(crate) fn follow_by(&mut self, edges: u64, clocks: &Clocks) -> Result<(), Error> {
        let WaitState::Running { target, .. } = self.state else {
            return Err(Error::Internal("skipped edges of an unscheduled wait"));
        };
        let target = target.checked_add(edges).ok_or(TimeError::Overflow)?;
        self.state = WaitState::Running {
            target,
            cached: if clocks.available(self.tap) {
                clocks.edge(target, self.tap)?
            } else {
                Time::MAX
            },
            revision: clocks.revision.0,
        };
        Ok(())
    }
    /// Continue a completed periodic wait on the same divider. Its ordinal
    /// identifies the consumed edge without converting its timestamp back to a count.
    pub(crate) fn follow(&mut self, clocks: &Clocks) -> Result<(), Error> {
        let target = match self.state {
            WaitState::Running { target, .. } => {
                target.checked_add(1).ok_or(TimeError::Overflow)?
            }
            WaitState::Ready(at) => {
                *self = Self::after(at, 1, self.tap, clocks)?;
                return Ok(());
            }
            WaitState::Paused { .. } => return Err(Error::Internal("unfinished periodic wait")),
        };
        let cached = if clocks.available(self.tap) {
            clocks.edge(target, self.tap)?
        } else {
            Time::MAX
        };
        self.state = WaitState::Running {
            target,
            cached,
            revision: clocks.revision.0,
        };
        Ok(())
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

impl Tap {
    pub(crate) fn validate(self) -> Result<(), Error> {
        crate::state::require(self.divide != 0, "zero saved clock divider")
    }
}
impl Clocks {
    pub(crate) fn validate(&mut self, now: Time) -> Result<(), Error> {
        use crate::state::require;
        for domain in [
            &self.system,
            &self.cpu,
            &self.watch,
            &self.on_chip,
            &self.oscillator,
            &self.subclock,
            &self.watch_crystal,
        ] {
            domain.validate(now)?;
        }
        require(
            self.frequencies.main_hz > 0
                && self.frequencies.watch_hz > 0
                && self.frequencies.on_chip_hz > 0
                && self.subclock_divide > 0
                && self.system_divide > 0
                && self.system_numerator > 0
                && self.system_denominator > 0
                && matches!(
                    self.system_source,
                    Source::Oscillator | Source::OnChip | Source::Watch
                ),
            "invalid clock routes",
        )?;
        self.prescaler_s
            .validate(self.raw_ticks(now, Source::System))?;
        self.prescaler_w
            .validate(self.raw_ticks(now, Source::Watch) / 4)?;
        self.revision = Revision(fresh_revision());
        self.lock = LockMemo::default();
        Ok(())
    }
}
