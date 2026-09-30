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
    #[cfg(feature = "profile-work")]
    pub(crate) time_materializations: crate::profile_work::Counter,
}
impl CpuWindow {
    pub fn at(&self) -> Result<Time, Error> {
        #[cfg(feature = "profile-work")]
        self.time_materializations.add(1);
        Ok(self.start.after(self.elapsed)?)
    }
    pub fn before(&self, edges: u64) -> Result<Time, Error> {
        #[cfg(feature = "profile-work")]
        self.time_materializations.add(1);
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
    pub fn finish(self, cursor: &mut CpuCursor) -> Result<(), Error> {
        cursor.clock = self.start;
        cursor.clock.advance(self.elapsed)?;
        Ok(())
    }
}
impl CpuCursor {
    pub fn new(clocks: &Clocks) -> Self {
        Self {
            clock: clocks.cpu.clock,
            revision: clocks.revision,
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
        if self.revision != clocks.revision || self.clock.at > now {
            self.clock = clocks.cpu.clock;
            self.revision = clocks.revision;
        }
        if self.clock.at < now {
            let elapsed = self
                .clock
                .edges_before(Time::from_raw(now.raw().saturating_add(1)));
            #[cfg(feature = "profile-work")]
            clocks.cpu_time_materializations.add(1);
            self.clock.advance(elapsed)?;
        }
        #[cfg(feature = "profile-work")]
        clocks.cpu_time_materializations.add(1);
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
            available: self.clock.edges_before(end),
            #[cfg(feature = "profile-work")]
            time_materializations: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcu::clocks::{Frequencies, Source};

    #[test]
    fn cursor_preserves_obligations_across_interruptions_and_clock_changes() {
        let mut clocks = Clocks::new(Time::ZERO, Frequencies::default()).unwrap();
        let mut cursor = CpuCursor::new(&clocks);
        let mut now = Time::ZERO;
        for i in 0..1000 {
            match i % 13 {
                0 => clocks.select_system(now, Source::Oscillator, 8).unwrap(),
                1 => clocks.select_subclock(now, 2).unwrap(),
                2 => clocks.select_cpu(now, true).unwrap(),
                3 => clocks.select_cpu(now, false).unwrap(),
                4 => clocks.select_system(now, Source::OnChip, 1).unwrap(),
                5 => clocks.select_system(now, Source::Watch, 2).unwrap(),
                6 => cursor = CpuCursor::new(&clocks),
                _ => {}
            }
            if i % 17 == 0 {
                // An issued access can be abandoned by reset before it completes.
                cursor.after(now, 7, &clocks).unwrap();
            }
            let edges = i % 21;
            let expected = ClockWait::after(now, edges, Tap::cpu(), &clocks).unwrap();
            let actual = cursor.after(now, edges, &clocks).unwrap();
            assert_eq!(actual, expected);
            now = actual.deadline(&clocks).unwrap().unwrap();
            if i % 7 == 0 {
                now = now.checked_add(crate::Duration::from_micros(2000)).unwrap();
            }
        }
    }
}
