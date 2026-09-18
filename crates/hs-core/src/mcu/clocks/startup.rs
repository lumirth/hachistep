//! Qualified source availability after physical supply loss or oscillator stop.
use super::SourcePower;
use crate::{
    error::Error,
    time::{Duration, Time, TimeError},
};

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum State {
    Stopped = 0,
    Starting(Time) = 1,
    Ready = 2,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Startup {
    rail: u16,
    sources: [State; 3], // main crystal, ROSC, watch crystal
    cold_main: bool,
}
impl Default for Startup {
    fn default() -> Self {
        Self {
            rail: 3000,
            sources: [State::Ready; 3],
            cold_main: false,
        }
    }
}
impl Startup {
    pub fn supplied(&self) -> bool {
        self.rail >= 1800
    }
    pub fn set_rail(&mut self, rail: u16) {
        self.rail = rail;
        if !self.supplied() {
            self.sources = [State::Stopped; 3];
            self.cold_main = true;
        }
    }
    pub fn deadline(&self) -> Option<Time> {
        self.sources
            .iter()
            .filter_map(|s| match s {
                State::Starting(at) => Some(*at),
                _ => None,
            })
            .min()
    }
    pub fn qualify(
        &mut self,
        mut request: SourcePower,
        now: Time,
        rosc_main: bool,
    ) -> Result<SourcePower, Error> {
        let demanded = [request.oscillator, request.on_chip, request.crystal];
        for (i, demand) in demanded.into_iter().enumerate() {
            if !demand || !self.supplied() {
                self.sources[i] = State::Stopped;
                continue;
            }
            if self.sources[i] == State::Stopped {
                let us = match i {
                    0 if !self.cold_main => 0,
                    0 if self.rail >= 2700 => 300,
                    0 if self.rail >= 2200 => 600,
                    0 => 50000,
                    1 => 15,
                    _ if self.rail >= 2200 => 2_000_000,
                    _ => 4_000_000,
                };
                self.sources[i] = State::Starting(
                    now.checked_add(Duration::from_micros(us))
                        .ok_or(TimeError::Overflow)?,
                );
            }
            if matches!(self.sources[i], State::Starting(at) if at <= now) {
                self.sources[i] = State::Ready;
                if i == 0 {
                    self.cold_main = false;
                }
            }
        }
        request.oscillator &= self.sources[0] == State::Ready;
        request.on_chip &= self.sources[1] == State::Ready;
        request.crystal &= self.sources[2] == State::Ready;
        request.main &= if rosc_main {
            request.on_chip
        } else {
            request.oscillator
        };
        request.watch &= if request.watch_on_chip {
            request.on_chip
        } else {
            request.crystal
        };
        Ok(request)
    }
}

impl Startup {
    pub(crate) fn validate(&self, now: Time) -> Result<(), Error> {
        crate::state::future(self.deadline(), now)
    }
}
