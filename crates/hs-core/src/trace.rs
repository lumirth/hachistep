//! Explicit observation of committed CPU bus accesses.
use crate::{Error, Machine, Output, RunResult, Time, TimedInput};
use core::ops::ControlFlow;

/// One completed physical CPU access. A word access to byte-wide registers
/// produces one record per completed byte lane. Idle cycles produce no record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusEvent {
    pub at: Time,
    /// Instruction owning the access, including instruction fetches.
    pub pc: u16,
    pub address: u16,
    /// Physical access width in bytes: one or two.
    pub width: u8,
    pub write: bool,
    /// Value read or written, after the physical access has committed.
    pub value: u16,
}

/// Borrowed synchronous bus observer. The observer must not re-enter the machine.
/// `Break(())` requests control after all effects at the current instant finish,
/// with the same exclusive-horizon rule as [`Output`]. Retain host errors in the
/// observer; requesting control does not fault or enter captured hardware state.
pub trait BusTrace {
    fn event(&mut self, event: BusEvent) -> ControlFlow<()>;
}
impl<F: FnMut(BusEvent) -> ControlFlow<()>> BusTrace for F {
    fn event(&mut self, event: BusEvent) -> ControlFlow<()> {
        self(event)
    }
}
impl BusTrace for Vec<BusEvent> {
    fn event(&mut self, event: BusEvent) -> ControlFlow<()> {
        self.push(event);
        ControlFlow::Continue(())
    }
}

/// Advance with explicit bus observation through the ordinary hardware executor.
/// Product events remain in `out`; bus records go only to `trace`. Either sink
/// can request control, reported as `StopReason::Output`. Detailed observation
/// costs more than ordinary execution. Merely enabling the Cargo feature does
/// not enable observation for [`Machine::run_until`].
pub fn run_until_traced(
    machine: &mut Machine,
    end: Time,
    inputs: &[TimedInput],
    out: &mut dyn Output,
    trace: &mut dyn BusTrace,
) -> Result<RunResult, Error> {
    machine.run_observed(end, inputs, out, Some(trace))
}
