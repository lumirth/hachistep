//! Runnable, deterministic, single-engine Pokéwalker development core.
//!
//! Hardware coverage and unmeasured timing witnesses are documented in
//! `docs/STATUS.md` in the repository; running retail firmware is not a claim
//! of complete silicon accuracy.
//!
//! ```
//! use hs_core::{Images, Machine, Time};
//! // An original two-byte BRA-self diagnostic, not a Nintendo firmware image.
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
pub mod cpu;
pub mod devices;
pub mod error;
pub mod mcu;
pub mod signals;
pub mod time;
pub use error::Error;
pub use signals::{Acceleration, AnalogPin, Buttons, Event, Input, Output, TimedInput};
pub use time::{Duration, Time};

pub mod machine;
pub use machine::{Conditions, Images, Machine, RunResult, Snapshot, Statistics};
