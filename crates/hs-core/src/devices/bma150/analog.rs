//! Continuous response before ADC sampling. Bosch specifies a second-order
//! 1500-Hz stage; the model uses Butterworth damping with unity DC gain.
use crate::{time::TimeError, Error, Time};

const SIGNAL: i128 = 1 << 24; // micro-g, retaining sub-ADC physical response
const Q: i128 = 1 << 48;
const K: u128 = 28_623_055_379_020; // Q32: sqrt(2) * pi * 1500 per second
const PERIOD: u128 = (1u128 << 64) / 3000;
const COMMON: (i64, i64) = decay(PERIOD);

const fn round(n: i128, d: i128) -> i128 {
    (n + if n < 0 { -d / 2 } else { d / 2 }) / d
}

/// Evaluate exp((-1+i)*k*dt) in Q48 integer arithmetic. Reduce the argument
/// before the fixed Taylor polynomial, then restore its scale by squaring.
const fn decay(dt: u128) -> (i64, i64) {
    if dt >= (8u128 << 64) / 1000 {
        return (0, 0); // Below one signal bit even at the allowed input extremes.
    }
    let mut angle = ((dt * K) >> 48) as i128;
    let mut squares = 0;
    while angle > Q / 16 {
        angle /= 2;
        squares += 1;
    }
    let (mut re, mut im, mut tr, mut ti) = (Q, 0, Q, 0);
    let mut n = 1;
    while n <= 12 {
        let next = round((-tr - ti) * angle, Q * n);
        ti = round((tr - ti) * angle, Q * n);
        tr = next;
        re += tr;
        im += ti;
        n += 1;
    }
    while squares > 0 {
        let next = round(re * re - im * im, Q);
        im = round(2 * re * im, Q);
        re = next;
        squares -= 1;
    }
    (re as i64, im as i64)
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Axis {
    at: Time,
    output: i64,
    velocity: i64, // derivative divided by k; same Q24 micro-g units
}

impl Axis {
    pub(super) fn new(now: Time, input: i32) -> Self {
        Self {
            at: now,
            output: i64::from(input) << 24,
            velocity: 0,
        }
    }

    pub(super) fn advance(&mut self, now: Time, input: i32) -> Result<i64, Error> {
        let dt = now
            .duration_since(self.at)
            .ok_or(TimeError::Reversed)?
            .raw();
        let target = i128::from(input) * SIGNAL;
        if dt != 0 && (i128::from(self.output) != target || self.velocity != 0) {
            let (r, i) = if (PERIOD..=PERIOD + 1).contains(&dt) {
                COMMON
            } else {
                decay(dt)
            };
            let (r, i) = (i128::from(r), i128::from(i));
            let delta = i128::from(self.output) - target;
            let velocity = i128::from(self.velocity);
            self.output = (target + round((r + i) * delta + i * velocity, Q)) as i64;
            self.velocity = round((r - i) * velocity - 2 * i * delta, Q) as i64;
        }
        self.at = now;
        Ok(round(i128::from(self.output), SIGNAL) as i64)
    }

    pub(super) fn resume(&mut self, now: Time) {
        self.at = now;
    }

    pub(super) fn validate(&self, now: Time) -> Result<(), Error> {
        let y = i128::from(self.output);
        let z = y + i128::from(self.velocity);
        let limit = 4_000_000_000 * SIGNAL;
        crate::state::require(
            self.at <= now
                && self.output.unsigned_abs() <= (8_000_000_000u64 << 24)
                && self.velocity.unsigned_abs() <= (8_000_000_000u64 << 24)
                && y * y + z * z <= limit * limit,
            "invalid sensor analog response",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn physical_step_and_frequency_response_match_the_chosen_two_poles() {
        let k = 2f64.sqrt() * PI * 1500.0;
        for us in [1, 10, 50, 83, 166, 333, 500, 667, 1000, 2000, 8000] {
            let mut axis = Axis::new(Time::ZERO, 0);
            let actual = axis.advance(Time::from_micros(us), 1_000_000).unwrap();
            let a = k * us as f64 / 1_000_000.0;
            let expected = 1_000_000.0 * (1.0 - (-a).exp() * (a.cos() + a.sin()));
            assert!((actual as f64 - expected).abs() <= 1.0, "step at {us} us");
        }
        // Separate sinusoidal stimuli verify attenuation; the step does not fit
        // these results. Inputs are zero-order holds, so include their sinc.
        for hz in [300u128, 1500, 6000] {
            let mut axis = Axis::new(Time::ZERO, 0);
            let (mut sine, mut cosine) = (0.0, 0.0);
            for n in 1..=4096u128 {
                let phase = 2.0 * PI * n as f64 / 128.0;
                let input = (1_000_000.0 * (phase - 2.0 * PI / 128.0).sin()).round() as i32;
                let at = Time::from_raw((n << 64) / (128 * hz));
                let value = axis.advance(at, input).unwrap() as f64;
                axis.validate(at).unwrap();
                if n > 3072 {
                    sine += value * phase.sin();
                    cosine += value * phase.cos();
                }
            }
            let amplitude = 2.0 * sine.hypot(cosine) / (1024.0 * 1_000_000.0);
            let hold = (PI / 128.0).sin() / (PI / 128.0);
            let expected = hold / (1.0 + (hz as f64 / 1500.0).powi(4)).sqrt();
            assert!(
                (amplitude - expected).abs() < 0.0001,
                "{hz} Hz: {amplitude} / {expected}"
            );
        }
    }

    #[test]
    fn input_extremes_remain_bounded_and_saved_state_rejects_excess_energy() {
        let mut axis = Axis::new(Time::ZERO, 0);
        for n in 1..=4000 {
            let at = Time::from_micros(n * 17);
            axis.advance(
                at,
                if n % 31 < 15 {
                    1_000_000_000
                } else {
                    -1_000_000_000
                },
            )
            .unwrap();
            axis.validate(at).unwrap();
        }
        axis.output = 4_000_000_000i64 << 24;
        axis.velocity = axis.output;
        assert!(axis.validate(Time::MAX).is_err());
        assert!(Axis::new(Time::from_micros(1), 0)
            .validate(Time::ZERO)
            .is_err());
    }
}
