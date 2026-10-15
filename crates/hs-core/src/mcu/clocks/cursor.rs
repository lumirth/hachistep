use super::{ClockWait, Clocks, Tap, WaitState};
use crate::{time::Clock, Error, Time, TimeError};

/// Local cursor for consecutive CPU accesses. Reuse the rational clock phase
/// while the CPU runs, and reproject it after a clock change or interruption.
pub(crate) struct CpuCursor {
    clock: Clock,
    revision: u64,
}
/// An interval with unchanged clock configuration and interrupt sources.
/// Count local edges here; project time only when another owner observes them.
pub(crate) struct CpuWindow {
    start: Clock,
    elapsed: u64,
    available: u64,
}
impl CpuWindow {
    pub fn at(&self) -> Result<Time, Error> {
        Ok(self.start.after(self.elapsed)?)
    }
    /// When the window's `elapsed`-th edge ends.
    pub fn time_of(&self, elapsed: u64) -> Result<Time, Error> {
        Ok(self.start.after(elapsed)?)
    }
    /// Time of an absolute CPU edge on this unchanged clock projection.
    pub fn time_of_ordinal(&self, ordinal: u64) -> Option<Result<Time, Error>> {
        Some(self.time_of(ordinal.checked_sub(self.start.ordinal())?))
    }
    /// Window edges that end strictly before `at`.
    pub fn edges_before(&self, at: Time) -> u64 {
        self.start.edges_before(at)
    }
    pub fn before(&self, edges: u64) -> Result<Time, Error> {
        Ok(self.start.after(self.elapsed - edges)?)
    }
    pub fn advance(&mut self, edges: u64) -> Result<bool, Error> {
        let elapsed = self.elapsed.checked_add(edges).ok_or(TimeError::Overflow)?;
        self.start
            .ordinal()
            .checked_add(elapsed)
            .ok_or(TimeError::Overflow)?;
        self.elapsed = elapsed;
        Ok(self.elapsed <= self.available)
    }
    pub fn elapsed(&self) -> u64 {
        self.elapsed
    }
    /// Edges that complete strictly inside the window, counted from its start.
    pub fn available(&self) -> u64 {
        self.available
    }
    /// Account for edges spent by work that checked `available` itself.
    pub fn set_elapsed(&mut self, elapsed: u64) {
        debug_assert!(elapsed <= self.available);
        self.elapsed = elapsed;
    }
    /// The CPU clock's edge ordinal at the window's position.
    pub fn position(&self) -> u64 {
        self.start.ordinal() + self.elapsed
    }
    /// Edges that can still complete strictly inside the window.
    pub fn remaining(&self) -> u64 {
        self.available.saturating_sub(self.elapsed)
    }
    /// Count edges that complete strictly inside the window, or none.
    #[inline(always)]
    pub fn spend(&mut self, edges: u64) -> bool {
        let elapsed = self.elapsed + edges;
        if elapsed > self.available {
            return false;
        }
        self.elapsed = elapsed;
        true
    }
    /// Move the exclusive end of the window.
    pub fn bound(&mut self, end: Time) {
        self.available = Self::edges(&self.start, end);
    }
    /// Edges before `end`, limited so that no count inside the window
    /// overflows the clock's ordinal.
    fn edges(start: &Clock, end: Time) -> u64 {
        start
            .edges_before(end)
            .min((u64::MAX >> 1) - start.ordinal().min(u64::MAX >> 1))
    }
    /// Admit no further edges.
    pub fn close(&mut self) {
        self.available = self.elapsed;
    }
    /// Withdraw edges of an access that was not started.
    pub fn retract(&mut self, edges: u64) {
        self.elapsed -= edges;
    }
    pub fn finish(&self, cursor: &mut CpuCursor) -> Result<(), Error> {
        cursor.clock = self.start;
        cursor.clock.advance(self.elapsed)?;
        Ok(())
    }
}
impl CpuCursor {
    pub fn new(clocks: &Clocks) -> Self {
        Self {
            clock: clocks.cpu.clock,
            revision: clocks.revision(),
        }
    }
    pub fn after(&mut self, now: Time, edges: u64, clocks: &Clocks) -> Result<ClockWait, Error> {
        if edges == 0 || !clocks.cpu.running {
            return ClockWait::after(now, edges, Tap::cpu(), clocks);
        }
        self.advance(now, edges, clocks)?;
        Ok(self.wait())
    }
    pub fn advance(&mut self, now: Time, edges: u64, clocks: &Clocks) -> Result<Time, Error> {
        if self.revision != clocks.revision() || self.clock.at > now {
            self.clock = clocks.cpu.clock;
            self.revision = clocks.revision();
        }
        if self.clock.at < now {
            let elapsed = self
                .clock
                .edges_before(Time::from_raw(now.raw().saturating_add(1)));
            self.clock.advance(elapsed)?;
        }
        Ok(self.clock.advance(edges)?)
    }
    pub fn wait(&self) -> ClockWait {
        ClockWait {
            tap: Tap::cpu(),
            state: WaitState::Running {
                target: self.clock.ordinal(),
                cached: self.clock.at,
                revision: self.revision,
            },
        }
    }
    pub fn window(&mut self, now: Time, end: Time, clocks: &Clocks) -> Result<CpuWindow, Error> {
        self.advance(now, 0, clocks)?;
        Ok(CpuWindow {
            start: self.clock,
            elapsed: 0,
            available: CpuWindow::edges(&self.clock, end),
        })
    }
}
