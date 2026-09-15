use crate::time::{Time, TimeError};
use core::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    ImageSize { name: &'static str, expected: usize, actual: usize },
    PersistentStatus(u8),
    Decode { pc: u16, words: [u16; 5], count: u8 },
    Unsupported { component: &'static str, detail: &'static str, address: u16 },
    Unmapped { address: u16, write: bool, width: u8 },
    Time(TimeError),
    PastInput { now: Time, requested: Time },
    BadInput(&'static str),
    Internal(&'static str),
    Snapshot(&'static str),
}
impl From<TimeError> for Error { fn from(e: TimeError) -> Self { Self::Time(e) } }
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ImageSize { name, expected, actual } => write!(f, "{name}: expected {expected} bytes, got {actual}"),
            Self::PersistentStatus(v) => write!(f, "EEPROM persistent status contains nonpersistent bits: {v:02x}"),
            Self::Decode { pc, words, count } => write!(f, "unsupported/invalid H8 form at {pc:04x}: {:04x?}", &words[..usize::from(*count)]),
            Self::Unsupported { component, detail, address } => write!(f, "{component} at {address:04x}: {detail}"),
            Self::Unmapped { address, write, width } => write!(f, "uncharacterized {}-byte {} at {address:04x}", width, if *write { "write" } else { "read" }),
            Self::Time(v) => write!(f, "time error: {v:?}"),
            Self::PastInput { now, requested } => write!(f, "past input/horizon {requested:?}; machine is at {now:?}"),
            Self::BadInput(v) | Self::Internal(v) | Self::Snapshot(v) => f.write_str(v),
        }
    }
}
impl std::error::Error for Error {}
