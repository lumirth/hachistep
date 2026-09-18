//! The common rail, RES capacitor and volatile-cell retention. See
//! docs/research/power-and-reset.md for nominal component values.
use crate::{
    error::Error,
    time::{Duration, Time, TimeError},
};

const RETENTION: u128 = 15u128 << 64; // 15,000 mV ms
const Q: u64 = 1 << 32; // capacitor voltage in Q32 millivolts

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Power {
    pub rail: u16,
    capacitor: u64,
    at: Time,
    drive: Option<bool>,
    crossing: Option<Time>,
    dose: u128,
    dose_at: Time,
    loss: Option<Time>,
    lost: bool,
}
impl Power {
    /// Construction starts at an already energized, reset-qualified board.
    /// A zero-volt initial rail instead starts with a discharged capacitor.
    pub fn new(rail: u16) -> Result<Self, Error> {
        let mut p = Self {
            rail,
            capacitor: u64::from(rail) * Q,
            at: Time::ZERO,
            drive: None,
            crossing: None,
            dose: 0,
            dose_at: Time::ZERO,
            loss: None,
            lost: rail == 0,
        };
        p.schedule(Time::ZERO)?;
        Ok(p)
    }
    pub fn mcu(&self) -> bool {
        self.rail >= 1800
    }
    pub fn eeprom(&self) -> bool {
        self.rail >= 1800
    }
    fn voltage(&self, now: Time) -> u64 {
        if let Some(high) = self.drive {
            return if high { u64::from(self.rail) * Q } else { 0 };
        }
        let rail = u64::from(self.rail) * Q;
        if self.capacitor == rail {
            return rail;
        }
        let decay = decay(now.raw().saturating_sub(self.at.raw()));
        let delta = ((u128::from(self.capacitor.abs_diff(rail)) * u128::from(decay)) >> 62) as u64;
        if self.capacitor > rail {
            rail + delta
        } else {
            rail - delta
        }
    }
    pub fn reset_low(&self, now: Time) -> bool {
        self.voltage(now) * 5 < u64::from(self.rail) * Q * 4 || self.drive == Some(false)
    }
    fn exposure(&self, now: Time) -> u128 {
        self.dose
            .saturating_add(
                now.raw()
                    .saturating_sub(self.dose_at.raw())
                    .saturating_mul(u128::from(1500u16.saturating_sub(self.rail))),
            )
            .min(RETENTION)
    }
    pub fn set_rail(&mut self, rail: u16, now: Time) -> Result<(), Error> {
        if self.rail == rail {
            return Ok(());
        }
        self.capacitor = self.voltage(now);
        self.at = now;
        self.dose = self.exposure(now);
        self.dose_at = now;
        self.rail = rail;
        if rail >= 1500 {
            self.dose = 0;
            self.lost = false;
        }
        self.schedule(now)
    }
    /// An electrical fixture drives the actual package level, overriding RC.
    pub fn drive_reset(&mut self, high: bool, now: Time) -> Result<(), Error> {
        self.drive = Some(high);
        self.capacitor = if high { u64::from(self.rail) * Q } else { 0 };
        self.at = now;
        self.schedule(now)
    }
    fn schedule(&mut self, now: Time) -> Result<(), Error> {
        self.crossing = None;
        if self.mcu() && self.drive.is_none() && self.reset_low(now) {
            // The node charges monotonically toward this constant rail.
            // Search fixed-point time once per rail segment, never per CPU step.
            let (mut lo, mut hi) = (
                now.raw(),
                now.checked_add(Duration::from_millis(640))
                    .ok_or(TimeError::Overflow)?
                    .raw(),
            );
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if self.reset_low(Time::from_raw(mid)) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            self.crossing = Some(Time::from_raw(lo));
        }
        self.loss = if self.rail < 1500 && !self.lost {
            let remaining = RETENTION - self.exposure(now);
            let rate = u128::from(1500 - self.rail);
            Some(
                now.checked_add(Duration::from_raw(remaining.div_ceil(rate)))
                    .ok_or(TimeError::Overflow)?,
            )
        } else {
            None
        };
        Ok(())
    }
    pub fn deadline(&self) -> Option<Time> {
        [self.crossing, self.loss].into_iter().flatten().min()
    }
    /// Returns true only at the destructive retention-loss boundary.
    pub fn advance(&mut self, now: Time) -> bool {
        if self.crossing == Some(now) {
            self.crossing = None;
        }
        if self.loss != Some(now) {
            return false;
        }
        self.loss = None;
        self.dose = RETENTION;
        self.dose_at = now;
        self.lost = true;
        true
    }
}

/// exp(-t/10ms), Q62. Range reduction bounds the Taylor argument by 1/16;
/// twelve terms and integer squaring avoid host floating-point variation.
fn decay(elapsed: u128) -> u64 {
    let tau = Duration::from_millis(10).raw();
    if elapsed >= 64 * tau {
        return 0;
    }
    let mut x = (elapsed << 62) / tau;
    let mut squares = 0;
    while x > 1u128 << 58 {
        x >>= 1;
        squares += 1;
    }
    let mut term = 1i128 << 62;
    let mut sum = term;
    for n in 1..=12 {
        term = -((term * x as i128) >> 62) / n;
        sum += term;
    }
    let mut value = sum as u128;
    for _ in 0..squares {
        value = (value * value) >> 62;
    }
    value as u64
}

impl Power {
    pub(crate) fn validate(&self, now: Time) -> Result<(), Error> {
        crate::state::require(
            self.capacitor <= u64::from(u16::MAX) * Q
                && self.at <= now
                && self.dose_at <= now
                && self.dose <= RETENTION,
            "invalid rail progress",
        )?;
        crate::state::future(self.crossing, now)?;
        crate::state::future(self.loss, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rc_crossings_and_retention_follow_the_selected_circuit() {
        let mut cold = Power::new(0).unwrap();
        cold.set_rail(3000, Time::ZERO).unwrap();
        let cross = cold.deadline().unwrap();
        assert_eq!(cross.as_micros(), 16094);
        let ns = (cross.raw() * 1_000_000_000) >> 64;
        assert_eq!(ns, 16_094_379);
        assert!(cold.reset_low(Time::from_raw(cross.raw() - 1)));
        assert!(!cold.reset_low(cross));
        for (us, low, mv) in [(1000, false, 2714), (5000, true, 1819)] {
            let mut p = Power::new(3000).unwrap();
            p.set_rail(0, Time::ZERO).unwrap();
            assert_eq!(p.voltage(Time::from_micros(us)) / Q, mv);
            p.set_rail(3000, Time::from_micros(us)).unwrap();
            assert_eq!(p.reset_low(Time::from_micros(us)), low);
        }
        for (mv, us) in [(0, 10000), (1000, 30000), (1400, 150000)] {
            let mut p = Power::new(3000).unwrap();
            p.set_rail(mv, Time::ZERO).unwrap();
            let due = p.deadline().unwrap();
            // Raw-time ceiling can place a boundary one quantum past a
            // truncated microsecond constructor.
            assert!(due.raw().abs_diff(Time::from_micros(us).raw()) <= 1);
            assert!(!p.advance(Time::from_raw(due.raw() - 1)));
            assert!(p.advance(due));
        }
    }
}
