use crate::Id;
use anyhow::{ensure, Context, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use hs_core::{Event, Input, Machine, Output, Time, TimedInput};
use std::collections::VecDeque;
use std::ops::ControlFlow;

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
    pub sender: u8,
    pub incoming: [VecDeque<Edge>; 2],
    #[borsh(skip)]
    pub prediction: [Option<Box<Prediction>>; 2],
}

#[derive(Clone)]
pub(crate) struct Prediction {
    pub end: Time,
    pub edges: VecDeque<Edge>,
}

impl Link {
    pub fn new(ids: [Id; 2], origins: [Time; 2], emitting: [bool; 2]) -> Self {
        Self {
            ids,
            origins,
            elapsed: Time::ZERO,
            sender: 0,
            prediction: [None, None],
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
    pub fn inputs(&self, side: usize, end: Time) -> Vec<TimedInput> {
        self.incoming[side]
            .iter()
            .take_while(|edge| edge.at < end)
            .map(|edge| TimedInput {
                at: Time::from_raw(self.origins[side].raw() + edge.at.raw()),
                input: Input::InfraredLevel(edge.emitting),
            })
            .collect()
    }
    pub fn consume(&mut self, side: usize, end: Time) {
        let count = self.incoming[side].partition_point(|edge| edge.at < end);
        self.incoming[side].drain(..count);
    }
    pub fn transmit(&mut self, side: usize, events: impl Iterator<Item = Edge>) -> Result<()> {
        for edge in events {
            self.sender = side as u8;
            self.prediction[1 - side] = None;
            let at = edge
                .at
                .duration_since(self.origins[side])
                .ok_or_else(|| anyhow::anyhow!("Invalid optical timestamp"))?;
            // The exclusive run horizon is the next 64.64 instant after emission.
            let at = at.raw().checked_add(1).context("Optical clock overflow")?;
            let at = Time::from_raw(at);
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

pub(crate) fn predict(machine: &Machine, end: Time, inputs: &[TimedInput]) -> Result<Prediction> {
    struct Emissions(VecDeque<Edge>);
    impl Output for Emissions {
        fn exact_stops(&self) -> bool {
            false
        }
        fn event(&mut self, event: Event) -> ControlFlow<()> {
            if let Event::Infrared { at, emitting } = event {
                self.0.push_back(Edge { at, emitting });
            }
            ControlFlow::Continue(())
        }
    }
    let mut preview = machine.clone();
    let mut emissions = Emissions(VecDeque::new());
    preview.run_until(end, inputs, &mut emissions)?;
    Ok(Prediction {
        end,
        edges: emissions.0,
    })
}
