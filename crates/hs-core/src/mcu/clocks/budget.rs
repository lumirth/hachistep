use super::{ClockWait, Clocks, Tap, WaitState};
use crate::{time::Clock, Error, Time};

/// CPU work permitted before the next device, input, or caller boundary.
/// The local cursor is a disposable projection of the hardware clock phase.
pub(crate) struct CpuBudget {
    clock: Clock,
    revision: u64,
    end: Time,
    last: Option<u64>,
}
impl CpuBudget {
    pub fn new(clocks: &Clocks) -> Self {
        Self {
            clock: clocks.cpu.clock,
            revision: clocks.revision,
            end: Time::ZERO,
            last: None,
        }
    }
    pub fn bound(&mut self, end: Time, clocks: &Clocks) {
        self.end = end;
        self.last = if clocks.cpu.running {
            let clock = &clocks.cpu.clock;
            Some(clock.ordinal().saturating_add(clock.edges_before(end)))
        } else {
            None
        };
    }
    pub fn before_boundary(
        &self,
        wait: &ClockWait,
        clocks: &Clocks,
    ) -> Result<Option<Time>, Error> {
        debug_assert_eq!(wait.tap, Tap::cpu());
        match wait.state {
            WaitState::Running { target, .. } if self.last.is_some_and(|last| target <= last) => {
                wait.deadline(clocks)
            }
            WaitState::Ready(at) if at < self.end => Ok(Some(at)),
            _ => Ok(None),
        }
    }
    pub fn after(&mut self, now: Time, edges: u64, clocks: &Clocks) -> Result<ClockWait, Error> {
        if edges == 0 || !clocks.cpu.running {
            return ClockWait::after(now, edges, Tap::cpu(), clocks);
        }
        if self.revision != clocks.revision || self.clock.at > now {
            self.clock = clocks.cpu.clock;
            self.revision = clocks.revision;
        }
        if self.clock.at < now {
            let elapsed = self
                .clock
                .edges_before(Time::from_raw(now.raw().saturating_add(1)));
            self.clock.advance(elapsed)?;
        }
        let cached = self.clock.advance(edges)?;
        Ok(ClockWait {
            tap: Tap::cpu(),
            state: WaitState::Running {
                target: self.clock.ordinal(),
                cached,
                revision: clocks.revision,
            },
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
        let mut cursor = CpuBudget::new(&clocks);
        let mut now = Time::ZERO;
        for i in 0..1000 {
            match i % 13 {
                0 => clocks.select_system(now, Source::Oscillator, 8).unwrap(),
                1 => clocks.select_subclock(now, 2).unwrap(),
                2 => clocks.select_cpu(now, true).unwrap(),
                3 => clocks.select_cpu(now, false).unwrap(),
                4 => clocks.select_system(now, Source::OnChip, 1).unwrap(),
                5 => clocks.select_system(now, Source::Watch, 2).unwrap(),
                6 => cursor = CpuBudget::new(&clocks),
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

    #[test]
    fn cycle_budget_excludes_the_boundary_and_accepts_distant_horizons() {
        for hz in [7, 32_768, 3_686_400, u64::MAX] {
            let mut clocks = Clocks::new(
                Time::ZERO,
                Frequencies {
                    main_hz: hz,
                    ..Default::default()
                },
            )
            .unwrap();
            let now = clocks.after(Time::ZERO, 19, Tap::cpu()).unwrap();
            clocks.select_system(now, Source::Oscillator, 1).unwrap();
            let mut cursor = CpuBudget::new(&clocks);
            let wait = cursor.after(now, 7, &clocks).unwrap();
            let at = wait.deadline(&clocks).unwrap().unwrap();
            for (end, expected) in [
                (Time::from_raw(at.raw() - 1), None),
                (at, None),
                (Time::from_raw(at.raw() + 1), Some(at)),
                (Time::MAX, Some(at)),
            ] {
                cursor.bound(end, &clocks);
                assert_eq!(cursor.before_boundary(&wait, &clocks).unwrap(), expected);
            }
        }
    }
}
