//! SHA-256 input/output identity; the library uses the host's SHA instructions when
//! it has them and its portable code otherwise, with the same digests.
use sha2::{Digest, Sha256};

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
