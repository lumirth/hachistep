//! Deterministic Pokéwalker emulator core.
//!
//! See `docs/API.md` for embedding and `docs/SOURCES.md` for the hardware
//! evidence and selected model parameters.
//!
//! ```
//! use hs_core::{Images, Machine, Time};
//! // A two-byte BRA instruction that loops back to itself.
//! let mut rom = vec![0u8; 49_152];
//! rom[..2].copy_from_slice(&0x0100u16.to_be_bytes());
//! rom[0x100..0x102].copy_from_slice(&[0x40, 0xfe]);
//! let mut machine = Machine::new(Images {
//!     firmware: &rom, eeprom: &[0xff; 65_536], eeprom_status: 0,
//! })?;
//! machine.run_until(Time::from_micros(100), &[], &mut ())?;
//! assert!(machine.retired() > 0);
//! # Ok::<(), hs_core::Error>(())
//! ```
#![forbid(unsafe_code)]
pub mod audio;
pub mod cpu;
pub mod devices;
pub mod error;
pub mod mcu;
mod power;
pub mod signals;
mod state;
pub mod time;
pub use audio::Audio;
pub use error::Error;
pub use signals::{Acceleration, AnalogPin, Buttons, DigitalPin, Event, Input, Output, TimedInput};
pub use time::{Duration, Time};

pub mod machine;
pub use machine::{Conditions, Images, Machine, RunResult, Snapshot, Statistics};
