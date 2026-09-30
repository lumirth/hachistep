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
//! ```
//! use hs_core::{Images, Machine, Time};
//! // A two-byte BRA instruction that loops back to itself.
//! let mut rom = vec![0u8; 49_152];
//! rom[..2].copy_from_slice(&0x0100u16.to_be_bytes());
//! rom[0x100..0x102].copy_from_slice(&[0x40, 0xfe]);
//! let mut machine = Machine::new(Images {
//!     firmware: &rom, eeprom: &[0xff; 65_536], eeprom_status: 0,
//!     sensor_nonvolatile: None,
//! })?;
//! machine.run_until(Time::from_micros(100), &[], &mut ())?;
//! assert!(machine.retired() > 0);
//! # Ok::<(), hs_core::Error>(())
//! ```
#![forbid(unsafe_code)]
mod audio;
mod cpu;
mod devices;
mod error;
mod mcu;
mod power;
#[cfg(feature = "profile-work")]
mod profile_work;
mod serial;
mod signals;
mod state;
mod time;
pub use audio::Audio;
pub use cpu::Registers;
pub use devices::nt7508::LcdDrive;
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
    #[cfg(feature = "profile-work")]
    pub use crate::profile_work::{IntervalExits, Work};
    /// Host work frequencies; enabled separately from ordinary release timing.
    #[cfg(feature = "profile-work")]
    pub fn work(machine: &crate::Machine) -> Work {
        machine.work()
    }

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
