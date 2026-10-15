use super::{Action, Width, WriteOrigin};
use crate::Error;

pub(crate) enum Stop {
    /// A projection publishes this access instead of performing it.
    Request(Action),
    /// The access completes at or after the interval's horizon.
    Horizon(Action),
    /// Admission inside unfinished work needs the request offered later.
    Sample,
    Core(Error),
    Reset,
    /// The CPU clock stopped; no further access may start in this interval.
    Fence,
}
impl From<Error> for Stop {
    fn from(error: Error) -> Self {
        Self::Core(error)
    }
}

/// Physical operations in the order and with the provenance the CPU issues them.
pub(crate) trait Bus {
    fn read(&mut self, address: u16, width: Width, fetch: bool) -> Result<u16, Stop>;
    fn write(
        &mut self,
        address: u16,
        width: Width,
        value: u16,
        origin: WriteOrigin,
    ) -> Result<(), Stop>;
    fn idle(&mut self, states: u32) -> Result<(), Stop>;
    /// The request offered at an admission point inside unfinished work.
    fn sample(&mut self) -> Result<Option<u8>, Stop>;
    /// The CPU's position for recognizing repetition, at a work boundary.
    /// `None` when repeated work cannot be summarized here.
    fn mark(&self) -> Option<Mark> {
        None
    }
    /// The work since `since` restored the architectural state it began
    /// with. Account for as many further identical repetitions as certainly
    /// complete before anything else can change, and return their count.
    fn repeat(&mut self, _since: &Mark) -> u64 {
        0
    }
}
/// A bus position at a work boundary. Equal disturbance counts mean that no
/// access between two marks changed state, other than by writes that leave
/// the board as they found it when repeated, or observed state that can
/// change before the interval's boundary.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    /// CPU clock edges.
    pub position: u64,
    pub disturbances: u64,
    /// Completed physical accesses, for diagnostic totals.
    pub reads: u64,
    pub writes: u64,
}
impl Mark {
    /// Complete repetitions that fit the remaining edge budget. Writes may
    /// replace a time-dependent owner state (for example a watchdog reload),
    /// so leave their last iteration to materialize its effects at that time.
    #[inline]
    pub fn repetitions_since(&self, since: &Self, remaining: u64) -> u64 {
        let period = self.position.wrapping_sub(since.position);
        if period == 0 || self.disturbances != since.disturbances {
            return 0;
        }
        (remaining / period).saturating_sub(u64::from(self.writes != since.writes))
    }

    /// The reads and writes of `repetitions` more iterations from `since`.
    pub fn accesses_since(&self, since: &Mark, repetitions: u64) -> (u64, u64) {
        (
            repetitions.wrapping_mul(self.reads.wrapping_sub(since.reads)),
            repetitions.wrapping_mul(self.writes.wrapping_sub(since.writes)),
        )
    }
}
pub(crate) trait IntervalBus: Bus {
    /// Run `work` over memory for consecutive work whose accesses are all
    /// memory and complete before the horizon, while nothing else can
    /// change, with the window it runs in, when the interval allows it (see
    /// `cpu::direct`).
    fn direct(
        &mut self,
        work: impl FnOnce(&mut super::direct::Memory<'_>, &mut super::direct::DirectRun),
    ) -> Result<(), Error>;
    /// Whether the CPU may start another unit of work.
    fn proceed(&self) -> bool;
    fn interrupt(&self) -> Option<u8>;
    fn instruction_boundary(&mut self);
    /// Exception acceptance protects flash and acknowledges NMI before entry.
    fn accept(&mut self, vector: u8) -> Result<(), Stop>;
    /// Attribute subsequent traced accesses to this instruction.
    #[cfg(feature = "trace")]
    fn instruction(&mut self, pc: u16);
}

/// Publishes the next physical operation without performing it.
pub(crate) struct Projection {
    pub interrupt: Option<u8>,
    /// Whether admission inside unfinished work may be decided now.
    pub admit: bool,
}
impl Bus for Projection {
    fn read(&mut self, address: u16, width: Width, fetch: bool) -> Result<u16, Stop> {
        Err(Stop::Request(Action::Read {
            address,
            width,
            fetch,
        }))
    }
    fn write(
        &mut self,
        address: u16,
        width: Width,
        value: u16,
        origin: WriteOrigin,
    ) -> Result<(), Stop> {
        Err(Stop::Request(Action::Write {
            address,
            width,
            value,
            origin,
        }))
    }
    fn idle(&mut self, states: u32) -> Result<(), Stop> {
        Err(Stop::Request(Action::Idle(states)))
    }
    fn sample(&mut self) -> Result<Option<u8>, Stop> {
        if self.admit {
            Ok(self.interrupt)
        } else {
            Err(Stop::Sample)
        }
    }
}

pub(crate) enum Exit {
    /// The access completes at or after the interval's horizon.
    Horizon(Action),
    /// The CPU clock stopped before the next access or admission.
    Fence,
    Sleep,
    Reset,
}
