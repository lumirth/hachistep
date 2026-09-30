use super::{Action, Width};
use crate::Error;

pub(crate) enum Stop {
    Request(Action),
    Horizon(Action),
    Core(Error),
    Reset,
    CommittedOwner,
}
impl From<Error> for Stop {
    fn from(error: Error) -> Self {
        Self::Core(error)
    }
}

/// Physical operations are authored with their semantic completion in one CPU
/// arm. Adapters choose whether to supply, project, or perform the transaction.
pub(crate) trait Bus {
    /// Diagnostic replies complete exactly one issued effect.
    const RUN_AHEAD: bool = false;
    fn read(&mut self, address: u16, width: Width, fetch: bool) -> Result<u16, Stop>;
    fn write(&mut self, address: u16, width: Width, value: u16, mov_byte: bool)
        -> Result<(), Stop>;
    fn idle(&mut self, states: u32) -> Result<(), Stop>;
}
pub(crate) trait IntervalBus: Bus {
    fn interrupt(&self) -> Option<u8>;
    fn instruction_boundary(&mut self);
}

pub(crate) struct Projection;
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
        mov_byte: bool,
    ) -> Result<(), Stop> {
        Err(Stop::Request(Action::Write {
            address,
            width,
            value,
            mov_byte,
        }))
    }
    fn idle(&mut self, states: u32) -> Result<(), Stop> {
        Err(Stop::Request(Action::Idle(states)))
    }
}
pub(crate) struct Reply(pub u16);
impl Bus for Reply {
    fn read(&mut self, _: u16, _: Width, _: bool) -> Result<u16, Stop> {
        Ok(self.0)
    }
    fn write(&mut self, _: u16, _: Width, _: u16, _: bool) -> Result<(), Stop> {
        Ok(())
    }
    fn idle(&mut self, _: u32) -> Result<(), Stop> {
        Ok(())
    }
}

pub(crate) enum Exit {
    Request(Action),
    Horizon(Action),
    Exception(super::Request),
    Sleep,
    Reset,
    CommittedOwner,
}
