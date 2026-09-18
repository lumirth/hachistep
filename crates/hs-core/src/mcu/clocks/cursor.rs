use super::{ClockWait, Clocks, Tap, WaitState};
use crate::{time::Clock, Error, Time};

/// Local cursor for consecutive CPU accesses. Reuse the rational clock phase
/// while the CPU runs, and reproject it after a clock change or interruption.
pub(crate) struct CpuCursor {
    clock: Clock,
    revision: u64,
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
