//! Compact cell progression for an accepted EEPROM write. Erase/program is
//! physical; the equal phase split and fixed cell thresholds are the canonical
//! part model, not measured timing. See research/lcd-and-eeprom.md.
use crate::time::{Duration, Time, TimeError};

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WriteCycle {
    pub started: Time,
    pub deadline: Time,
}
impl WriteCycle {
    pub fn start(now: Time, duration: Duration) -> Result<Self, TimeError> {
        Ok(Self {
            started: now,
            deadline: now.checked_add(duration).ok_or(TimeError::Overflow)?,
        })
    }
    pub fn byte(self, old: u8, target: u8, cell: u32, now: Time) -> u8 {
        if now <= self.started {
            return old;
        }
        if now >= self.deadline {
            return target;
        }
        let duration = self.deadline.raw() - self.started.raw();
        let elapsed = now.raw() - self.started.raw();
        let erase = duration / 2;
        if elapsed < erase {
            old & !crossed(cell, elapsed, erase)
        } else {
            target & crossed(cell ^ 0x8a51_3c27, elapsed - erase, duration - erase)
        }
    }
}
fn crossed(cell: u32, elapsed: u128, duration: u128) -> u8 {
    let mut mask = 0;
    for bit in 0..8 {
        let mut key = cell ^ 0x9e37_79b9u32.wrapping_mul(bit + 1);
        key = (key ^ (key >> 16)).wrapping_mul(0x7feb_352d);
        key = (key ^ (key >> 15)).wrapping_mul(0x846c_a68b);
        let threshold = u128::from(((key ^ (key >> 16)) & 0xffff).max(1));
        // Quotient/remainder scaling avoids overflowing even a long duration.
        let at = duration / 65536 * threshold + duration % 65536 * threshold / 65536;
        if elapsed >= at {
            mask |= 1 << bit;
        }
    }
    mask
}

impl WriteCycle {
    pub(crate) fn validate(self, now: Time) -> Result<(), crate::Error> {
        crate::state::require(
            self.started <= now && self.deadline > self.started && self.deadline >= now,
            "invalid nonvolatile write interval",
        )
    }
}
