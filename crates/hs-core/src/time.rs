//! Host-independent time. The public timeline is 64.64 fixed-point seconds.
//! A clock retains its fractional remainder: partitioning an advance cannot
//! change its phase or cumulative frequency.
use core::fmt;

#[derive(
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
pub struct Time(pub(crate) u128);
#[derive(
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
pub struct Duration(pub(crate) u128);

impl Time {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u128::MAX);
    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }
    pub const fn raw(self) -> u128 {
        self.0
    }
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        self.0.checked_add(duration.0).map(Self)
    }
    pub fn duration_since(self, before: Self) -> Option<Duration> {
        self.0.checked_sub(before.0).map(Duration)
    }
    pub fn from_micros(us: u64) -> Self {
        Self((u128::from(us) << 64) / 1_000_000)
    }
    pub fn as_micros(self) -> u128 {
        (self.0 >> 64) * 1_000_000 + (((self.0 & u128::from(u64::MAX)) * 1_000_000) >> 64)
    }
}
impl Duration {
    pub const ZERO: Self = Self(0);
    pub const fn from_raw(raw: u128) -> Self {
        Self(raw)
    }
    pub const fn raw(self) -> u128 {
        self.0
    }
    pub const fn seconds(seconds: u64) -> Self {
        Self((seconds as u128) << 64)
    }
    pub fn from_micros(us: u64) -> Self {
        Self((u128::from(us) << 64) / 1_000_000)
    }
    pub fn from_millis(ms: u64) -> Self {
        Self((u128::from(ms) << 64) / 1000)
    }
}
impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{:06}s",
            self.as_micros() / 1_000_000,
            self.as_micros() % 1_000_000
        )
    }
}
impl fmt::Debug for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Duration({})", self.0)
    }
}

/// A running rational clock. `at` is the time of the last advanced edge, not
/// the host's latest observation time. Gate owners decide whether to retain or
/// reset phase; merely reading a clock never rephases it.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub(crate) at: Time,
    pub(crate) whole: u128,
    pub(crate) remainder: u64,
    pub(crate) denominator: u64,
    pub(crate) fraction: u64,
    pub(crate) ordinal: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    ZeroFrequency,
    UnrepresentableFrequency,
    Overflow,
    Reversed,
}
impl Clock {
    /// Frequency is numerator/denominator Hz. Both arguments must be nonzero.
    pub fn new(at: Time, numerator: u64, denominator: u64) -> Result<Self, TimeError> {
        if numerator == 0 || denominator == 0 {
            return Err(TimeError::ZeroFrequency);
        }
        let period = u128::from(denominator) << 64;
        let whole = period / u128::from(numerator);
        if whole == 0 {
            return Err(TimeError::UnrepresentableFrequency);
        }
        Ok(Self {
            at,
            whole,
            remainder: (period % u128::from(numerator)) as u64,
            denominator: numerator,
            fraction: 0,
            ordinal: 0,
        })
    }
    pub fn next(&self) -> Result<Time, TimeError> {
        self.after(1)
    }
    pub fn after(&self, edges: u64) -> Result<Time, TimeError> {
        let fractions = u128::from(self.fraction) + u128::from(edges) * u128::from(self.remainder);
        let delta = self
            .whole
            .checked_mul(u128::from(edges))
            .and_then(|v| v.checked_add(fractions / u128::from(self.denominator)))
            .ok_or(TimeError::Overflow)?;
        self.at
            .0
            .checked_add(delta)
            .map(Time)
            .ok_or(TimeError::Overflow)
    }
    pub fn advance(&mut self, edges: u64) -> Result<Time, TimeError> {
        let next = self.after(edges)?;
        self.ordinal = self.ordinal.checked_add(edges).ok_or(TimeError::Overflow)?;
        self.fraction = ((u128::from(self.fraction)
            + u128::from(edges) * u128::from(self.remainder))
            % u128::from(self.denominator)) as u64;
        self.at = next;
        Ok(next)
    }
    /// Count edges strictly before a horizon by inverting the same rational
    /// recurrence used by `after`. The normal calculation is O(1). The checked
    /// binary fallback handles enormous horizons without intermediate overflow;
    /// it is an exact arithmetic fallback, not another CPU execution engine.
    pub fn edges_before(&self, limit: Time) -> u64 {
        if limit <= self.at {
            return 0;
        }
        let delta = limit.0 - self.at.0;
        if let Some(scaled) = delta.checked_mul(u128::from(self.denominator)) {
            let p = self.whole * u128::from(self.denominator) + u128::from(self.remainder);
            return ((scaled - 1 - u128::from(self.fraction)) / p).min(u128::from(u64::MAX)) as u64;
        }
        let upper = ((delta - 1) / self.whole).min(u128::from(u64::MAX)) as u64;
        let (mut lo, mut hi) = (0u64, upper);
        while lo < hi {
            let mid = lo + (hi - lo) / 2 + 1;
            if self.after(mid).map(|t| t < limit).unwrap_or(false) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }
    pub fn rebase(&mut self, at: Time) {
        self.at = at;
    }
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }
}

impl Clock {
    pub(crate) fn validate(&self, now: Time) -> Result<(), crate::Error> {
        crate::state::require(
            self.whole > 0
                && self.denominator > 0
                && self.remainder < self.denominator
                && self.fraction < self.denominator
                && self
                    .whole
                    .checked_mul(u128::from(self.denominator))
                    .and_then(|v| v.checked_add(u128::from(self.remainder)))
                    .is_some()
                && self.at <= now,
            "invalid rational clock",
        )?;
        crate::state::require(
            self.ordinal
                .checked_add(self.edges_before(now))
                .and_then(|v| v.checked_add(65536))
                .is_some(),
            "clock ordinal overflow",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_one_second_and_partitioning() {
        for frequency in [3_686_400, 32_768, 3_000, 1_310_720, 7] {
            let mut bulk = Clock::new(Time::ZERO, frequency, 1).unwrap();
            let mut pieces = bulk;
            bulk.advance(frequency).unwrap();
            for _ in 0..frequency / 7 {
                pieces.advance(7).unwrap();
            }
            pieces.advance(frequency % 7).unwrap();
            assert_eq!(bulk, pieces);
            assert_eq!(bulk.at, Time(1u128 << 64));
        }
    }
    #[test]
    fn endpoints_are_exclusive() {
        let c = Clock::new(Time::ZERO, 32768, 1).unwrap();
        let e = c.after(12).unwrap();
        assert_eq!(c.edges_before(e), 11);
        assert_eq!(c.edges_before(Time(e.0 + 1)), 12);
    }
    #[test]
    fn rejects_bad_time_and_checks_overflow() {
        assert!(Clock::new(Time::ZERO, 0, 1).is_err());
        let c = Clock::new(Time::MAX, 1, 1).unwrap();
        assert_eq!(c.after(1), Err(TimeError::Overflow));
        assert!(Time::ZERO.duration_since(Time(1)).is_none());
    }
}

#[cfg(test)]
mod inversion_tests {
    use super::*;
    #[test]
    fn inversion_matches_enumerated_edges_after_fractional_advances() {
        for (n, d) in [(7, 3), (3_686_400, 1), (32768, 1), (3_000_001, 17)] {
            let mut c = Clock::new(Time::from_raw(12345), n, d).unwrap();
            c.advance(19).unwrap();
            for k in 1..1000 {
                let edge = c.after(k).unwrap();
                assert_eq!(c.edges_before(edge), k - 1);
                assert_eq!(c.edges_before(Time::from_raw(edge.raw() + 1)), k);
            }
        }
    }
}
