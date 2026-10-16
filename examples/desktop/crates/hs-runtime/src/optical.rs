use crate::Id;
use anyhow::{ensure, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use hs_core::{Duration, Input, Time, TimedInput};
use std::collections::VecDeque;

#[derive(Clone, BorshSerialize, BorshDeserialize)]
pub(crate) struct Edge {
    pub at: Time,
    pub emitting: bool,
}

#[derive(Clone, BorshSerialize, BorshDeserialize)]
pub(crate) struct Link {
    pub ids: [Id; 2],
    pub origins: [Time; 2],
    pub elapsed: Time,
    pub incoming: [VecDeque<Edge>; 2],
}

impl Link {
    pub fn new(ids: [Id; 2], origins: [Time; 2], emitting: [bool; 2]) -> Self {
        Self {
            ids,
            origins,
            elapsed: Time::ZERO,
            incoming: [
                VecDeque::from([Edge {
                    at: Time::ZERO,
                    emitting: emitting[1],
                }]),
                VecDeque::from([Edge {
                    at: Time::ZERO,
                    emitting: emitting[0],
                }]),
            ],
        }
    }
    pub fn deliver(&mut self, side: usize, end: Time) -> Vec<TimedInput> {
        let mut inputs = Vec::new();
        while self.incoming[side]
            .front()
            .is_some_and(|edge| edge.at < end)
        {
            let edge = self.incoming[side].pop_front().unwrap();
            inputs.push(TimedInput {
                at: Time::from_raw(self.origins[side].raw() + edge.at.raw()),
                input: Input::InfraredLevel(edge.emitting),
            });
        }
        inputs
    }
    pub fn transmit(&mut self, side: usize, events: impl Iterator<Item = Edge>) -> Result<()> {
        for edge in events {
            let at = edge
                .at
                .duration_since(self.origins[side])
                .ok_or_else(|| anyhow::anyhow!("Invalid optical timestamp"))?;
            let at = Time::from_raw(at.raw())
                .checked_add(Duration::from_micros(1))
                .ok_or_else(|| anyhow::anyhow!("Optical clock overflow"))?;
            ensure!(
                self.incoming[1 - side].len() < 65_536,
                "Optical queue overflow"
            );
            self.incoming[1 - side].push_back(Edge {
                at,
                emitting: edge.emitting,
            });
        }
        Ok(())
    }
}
