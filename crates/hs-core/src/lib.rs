//! Deterministic Pokéwalker emulator core.

//!
//! [`Machine`] owns one device. Supply timestamped [`TimedInput`] changes and
//! advance it through effects before a requested [`Time`]. Output events are
//! synchronous; a callback can request control through [`Output`]. The caller
//! owns host pacing, presentation, files and connection transport.
//!
//! Import embedding types from the crate root. [`diagnostic`] exposes component
//! interfaces for hardware experiments separately from this contract.
//!
//! See `examples/replay.rs` for a complete firmware caller.
#![forbid(unsafe_code)]
mod audio;
mod cpu;
mod devices;
mod error;
mod mcu;
mod power;
mod signals;
mod state;
mod time;
pub use audio::Audio;
pub use cpu::Registers;
pub use devices::nt7508::{LcdDrive, LcdVoltages};
pub use error::Error;
pub use mcu::clocks::Frequencies;
pub use signals::{
    Acceleration, AnalogPin, Buttons, DigitalPin, Event, Input, NvDomain, Output, Piezo, TimedInput,
};
pub use time::{Duration, Time, TimeError};

mod machine;
pub use machine::{Conditions, Images, Machine, RunResult, Snapshot, Statistics, StopReason};

/// Component-level access for diagnostic runners and hardware experiments.
///
/// These interfaces expose the hardware implementation and may change separately
/// from the machine embedding API. Applications should use the crate-root types.
pub mod diagnostic {
    /// Electrical level used by component-level pin fixtures.
    pub use crate::signals::Drive;
    /// H8 execution and instruction definitions for controlled CPU experiments.
    pub mod cpu {
        pub use crate::cpu::*;
    }
    /// Standalone external-device models for signal fixtures.
    pub mod devices {
        pub use crate::devices::*;
    }
    /// MCU components for register and timing experiments.
    pub mod mcu {
        pub use crate::mcu::*;
    }
    pub use crate::time::Clock;
}
