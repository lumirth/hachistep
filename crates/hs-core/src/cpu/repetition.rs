//! Recognition of work that repeats without effect, such as a loop polling a
//! status flag. Like the decode memo this is not state: it only lets the bus
//! account for repetitions that are proven identical, and copies start empty.
//!
//! A backward branch that returns to the architectural state it left, with
//! no access in between that introduced a new effect or observed state able
//! to change before the interval's boundary, starts an iteration that can
//! only repeat the last one exactly: the same instructions, the same
//! accesses at the same relative edges and the same values. The bus then accounts for whole
//! iterations up to the boundary instead of performing them. Iterations with
//! writes keep a final real iteration: replacement writes such as watchdog
//! reloads must leave their last effect at its own time (`Mark::repetitions_since`).
use super::{execution::Mark, Prefetch, Registers};

#[derive(Clone, Copy)]
struct Arrival {
    registers: Registers,
    prefetch: Prefetch,
    interrupt_delay: u8,
    mark: Mark,
    retired: u64,
}
pub(crate) struct Repetition {
    last: Option<Arrival>,
    /// The work just completed was a taken backward branch.
    pub branched: bool,
}
impl Repetition {
    pub fn new() -> Self {
        Self {
            last: None,
            branched: false,
        }
    }
    /// Forget arrivals; marks belong to one interval.
    pub fn forget(&mut self) {
        self.last = None;
        self.branched = false;
    }
    /// Record an arrival at a branch target. Returns the mark of the previous
    /// arrival and the work retired since it when this arrival repeats it.
    #[inline]
    pub fn arrive(
        &mut self,
        registers: &Registers,
        prefetch: Prefetch,
        interrupt_delay: u8,
        mark: Mark,
        retired: u64,
    ) -> Option<(Mark, u64)> {
        let repeated = self.last.as_ref().and_then(|last| {
            (last.mark.disturbances == mark.disturbances
                && last.registers == *registers
                && last.prefetch == prefetch
                && last.interrupt_delay == interrupt_delay)
                .then_some((last.mark, retired.wrapping_sub(last.retired)))
        });
        self.last = Some(Arrival {
            registers: *registers,
            prefetch,
            interrupt_delay,
            mark,
            retired,
        });
        repeated
    }
    /// The bus accounted for repetitions after the last arrival.
    pub fn advanced(&mut self, mark: Mark, retired: u64) {
        if let Some(last) = &mut self.last {
            last.mark = mark;
            last.retired = retired;
        }
    }
}
impl Clone for Repetition {
    fn clone(&self) -> Self {
        Self::new()
    }
}
impl PartialEq for Repetition {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Repetition {}
impl core::fmt::Debug for Repetition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Repetition")
    }
}
