//! Shared ripple-divider phase with monotonic edge ordinals. Resetting a
//! divider changes its phase without rewinding an owner's consumed-edge count.
use crate::{error::Error, time::TimeError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Prescaler<const N: usize> {
    anchor: u64,
    phase: u16,
    emitted: [u64; N],
    running: bool,
}
impl<const N: usize> Prescaler<N> {
    pub fn new() -> Self {
        Self {
            anchor: 0,
            phase: 0,
            emitted: [0; N],
            running: true,
        }
    }
    fn elapsed(&self, parent: u64) -> u64 {
        if self.running {
            parent.saturating_sub(self.anchor)
        } else {
            0
        }
    }
    pub fn ticks(&self, parent: u64, shift: u32) -> u64 {
        self.emitted[shift as usize - 1] + Self::crossings(self.phase, self.elapsed(parent), shift)
    }
    pub fn high(&self, parent: u64, shift: u32) -> bool {
        u64::from(self.phase).wrapping_add(self.elapsed(parent)) & (1 << (shift - 1)) == 0
    }
    pub fn parent_edge(&self, tick: u64, shift: u32) -> Result<u64, Error> {
        let count = tick
            .checked_sub(self.emitted[shift as usize - 1])
            .ok_or(TimeError::Reversed)?;
        let remainder = u64::from(self.phase) & ((1 << shift) - 1);
        self.anchor
            .checked_sub(remainder)
            .and_then(|v| count.checked_mul(1 << shift).and_then(|n| v.checked_add(n)))
            .ok_or_else(|| TimeError::Overflow.into())
    }
    fn crossings(phase: u16, elapsed: u64, shift: u32) -> u64 {
        let mask = (1 << shift) - 1;
        (elapsed >> shift) + (((u64::from(phase) & mask) + (elapsed & mask)) >> shift)
    }
    fn settle(&mut self, parent: u64) {
        let elapsed = self.elapsed(parent);
        for (i, emitted) in self.emitted.iter_mut().enumerate() {
            *emitted += Self::crossings(self.phase, elapsed, (i + 1) as u32);
        }
        self.phase = (u64::from(self.phase).wrapping_add(elapsed) & ((1 << N) - 1)) as u16;
        self.anchor = parent;
    }
    pub fn reset(&mut self, parent: u64) {
        self.settle(parent);
        self.phase = 0;
    }
    pub fn set_running(&mut self, running: bool, parent: u64, clear_on_stop: bool) -> bool {
        if self.running == running {
            return false;
        }
        self.settle(parent);
        self.running = running;
        if !running && clear_on_stop {
            self.phase = 0;
        }
        true
    }
}
