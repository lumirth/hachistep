//! A rational clock source advances only while enabled. Starting an oscillator
//! establishes a fresh phase; derived clocks join their source.
use crate::{
    error::Error,
    time::{Clock, Time},
};

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Domain {
    pub clock: Clock,
    pub running: bool,
    pub held_at: Time,
}
impl Domain {
    pub fn new(now: Time, hz: u64, divide: u64) -> Result<Self, Error> {
        Ok(Self {
            clock: Clock::new(now, hz, divide)?,
            running: true,
            held_at: now,
        })
    }
    pub fn time(&self, now: Time) -> Time {
        if self.running {
            now
        } else {
            self.held_at
        }
    }
    pub fn ticks(&self, now: Time) -> u64 {
        // stop() already settles the final source edge. A stopped derived
        // domain likewise starts with its count settled at held_at.
        if !self.running {
            return self.clock.ordinal();
        }
        self.clock.ordinal().saturating_add(
            self.clock
                .edges_before(Time::from_raw(now.raw().saturating_add(1))),
        )
    }
    pub fn stop(&mut self, now: Time) -> Result<(), Error> {
        if self.running {
            self.clock.advance(self.ticks(now) - self.clock.ordinal())?;
            self.running = false;
            self.held_at = now;
        }
        Ok(())
    }
    pub fn start(&mut self, now: Time, hz: u64) -> Result<(), Error> {
        let ordinal = self.ticks(now);
        *self = Self::new(now, hz, 1)?;
        self.clock.ordinal = ordinal;
        Ok(())
    }
    pub fn set_running(&mut self, running: bool, now: Time, hz: u64) -> Result<bool, Error> {
        if self.running == running {
            return Ok(false);
        }
        if running {
            self.start(now, hz)?;
        } else {
            self.stop(now)?;
        }
        Ok(true)
    }
}

impl Domain {
    pub(super) fn validate(&self, now: Time) -> Result<(), Error> {
        crate::state::require(
            self.held_at <= now && (self.running || self.held_at >= self.clock.at),
            "invalid held clock phase",
        )?;
        self.clock.validate(self.time(now))?;
        crate::state::require(
            self.running
                || self
                    .clock
                    .edges_before(Time::from_raw(self.held_at.raw().saturating_add(1)))
                    == 0,
            "stopped clock has unsettled edges",
        )
    }
}
