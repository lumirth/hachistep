//! Deterministic, single-engine Pokéwalker hardware model.
#![forbid(unsafe_code)]
pub mod cpu;
pub mod devices;
pub mod error;
pub mod signals;
pub mod time;
pub mod mcu;
pub use error::Error;
pub use signals::{Acceleration, Buttons, Event, Input, Output, TimedInput};
pub use time::{Duration, Time};

pub mod machine;
pub use machine::{Machine,Images,Conditions,RunResult,Snapshot,Statistics};
