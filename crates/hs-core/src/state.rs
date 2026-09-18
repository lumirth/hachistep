//! Private codec primitives. No guest execution or host I/O is used on load.
use crate::{Error, Time};
use std::io::{self, Read};

pub(crate) fn require(valid: bool, reason: &'static str) -> Result<(), Error> {
    if valid {
        Ok(())
    } else {
        Err(Error::Snapshot(reason))
    }
}
pub(crate) fn future(at: Option<Time>, now: Time) -> Result<(), Error> {
    require(
        at.is_none_or(|at| at >= now),
        "saved appointment is in the past",
    )
}

/// The length comes from the hardware, never from untrusted input.
pub(crate) fn read_bytes<const N: usize, R: Read>(reader: &mut R) -> io::Result<Box<[u8; N]>> {
    let mut bytes = vec![0; N].into_boxed_slice();
    reader.read_exact(&mut bytes)?;
    bytes
        .try_into()
        .map_err(|_| io::Error::other("fixed byte-array shape"))
}
