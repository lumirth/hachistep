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
        Self(micros_raw(us))
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
        Self(micros_raw(us))
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

/// `a * b`, unless it overflows, in two 64-bit multiplications.
#[inline]
fn widening_mul(a: u128, b: u64) -> Option<u128> {
    let low = u128::from(a as u64) * u128::from(b);
    let high = (a >> 64) * u128::from(b);
    if high >> 64 != 0 {
        return None;
    }
    (high << 64).checked_add(low)
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
        self.project(edges).map(|(at, _)| at)
    }
    #[inline]
    fn project(&self, edges: u64) -> Result<(Time, u64), TimeError> {
        let fractions = u128::from(self.fraction) + u128::from(edges) * u128::from(self.remainder);
        let (carry, fraction) = if edges <= 3 {
            // Short physical accesses and serial edges carry at most once per
            // edge. Subtraction avoids a division without changing the phase.
            let mut fraction = fractions;
            let mut carry = 0;
            while fraction >= u128::from(self.denominator) {
                fraction -= u128::from(self.denominator);
                carry += 1;
            }
            (carry, fraction as u64)
        } else if let Ok(fractions) = u64::try_from(fractions) {
            (
                u128::from(fractions / self.denominator),
                fractions % self.denominator,
            )
        } else {
            (
                fractions / u128::from(self.denominator),
                (fractions % u128::from(self.denominator)) as u64,
            )
        };
        let delta = widening_mul(self.whole, edges)
            .and_then(|v| v.checked_add(carry))
            .ok_or(TimeError::Overflow)?;
        self.at
            .0
            .checked_add(delta)
            .map(|at| (Time(at), fraction))
            .ok_or(TimeError::Overflow)
    }
    pub fn advance(&mut self, edges: u64) -> Result<Time, TimeError> {
        let (next, fraction) = self.project(edges)?;
        self.ordinal = self.ordinal.checked_add(edges).ok_or(TimeError::Overflow)?;
        self.fraction = fraction;
        self.at = next;
        Ok(next)
    }
    /// Count edges strictly before a horizon by inverting the same rational
    /// recurrence used by `after`. The normal calculation is O(1). The checked
    /// binary fallback handles enormous horizons without intermediate overflow.
    pub fn edges_before(&self, limit: Time) -> u64 {
        if limit <= self.at {
            return 0;
        }
        let delta = limit.0 - self.at.0;
        // The period numerator is an integer multiple of 2^64: new() starts
        // with denominator-Hz * 2^64, and dividers multiply it.
        // floor(n / (d * 2^64)) == floor((n >> 64) / d). Retaining the
        // fraction subtraction before shifting preserves exclusive edges.
        if let Some(scaled) = widening_mul(delta, self.denominator) {
            let p = widening_mul(self.whole, self.denominator)
                .and_then(|p| p.checked_add(u128::from(self.remainder)))
                .unwrap_or(u128::MAX);
            let edges = ((scaled - 1 - u128::from(self.fraction)) >> 64) as u64;
            let divide = (p >> 64) as u64;
            return if divide == 1 { edges } else { edges / divide };
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
                    .is_some_and(|period| period != 0 && period as u64 == 0)
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

/// `us` microseconds in 64.64 seconds, rounded down: the fraction by long
/// division in 32-bit digits, which stays in 64-bit arithmetic.
fn micros_raw(us: u64) -> u128 {
    const D: u64 = 1_000_000;
    let (whole, part) = (us / D, us % D);
    let high = (part << 32) / D;
    let low = (((part << 32) % D) << 32) / D;
    (u128::from(whole) << 64) | u128::from((high << 32) + low)
}
