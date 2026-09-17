//! Power-of-two moving averages over actual ADC samples. Retaining 64 samples
//! allows a bandwidth change to select a new window without inventing history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Filter {
    history: [[i16; 64]; 3],
    next: [u8; 3],
    count: [u8; 3],
    sum: [i32; 3],
    shift: u8,
}
impl Filter {
    pub(super) fn new(bandwidth: u8) -> Self {
        Self {
            history: [[0; 64]; 3],
            next: [0; 3],
            count: [0; 3],
            sum: [0; 3],
            shift: 6 - bandwidth.min(6),
        }
    }
    pub(super) fn window(&self) -> usize {
        1 << self.shift
    }
    pub(super) fn select(&mut self, bandwidth: u8) {
        let shift = 6 - bandwidth.min(6);
        if shift == self.shift {
            return;
        }
        self.shift = shift;
        for axis in 0..3 {
            self.sum[axis] = (0..self.window().min(usize::from(self.count[axis])))
                .map(|n| {
                    i32::from(self.history[axis][(usize::from(self.next[axis]) + 63 - n) & 63])
                })
                .sum();
        }
    }
    pub(super) fn push(&mut self, axis: usize, code: i16) -> i16 {
        let next = usize::from(self.next[axis]);
        let window = self.window();
        if usize::from(self.count[axis]) >= window {
            self.sum[axis] -= i32::from(self.history[axis][(next + 64 - window) & 63]);
        }
        self.sum[axis] += i32::from(code);
        self.history[axis][next] = code;
        self.next[axis] = ((next + 1) & 63) as u8;
        self.count[axis] = (self.count[axis] + 1).min(64);
        if usize::from(self.count[axis]) < window {
            code
        } else {
            (self.sum[axis] >> self.shift) as i16
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn step_impulse_startup_and_signed_rounding() {
        let mut f = Filter::new(4);
        assert_eq!([4, 8, 12, 16].map(|v| f.push(0, v)), [4, 8, 12, 10]);
        let mut f = Filter::new(4);
        for _ in 0..4 {
            f.push(0, 0);
        }
        assert_eq!([64, 64, 64, 64].map(|v| f.push(0, v)), [16, 32, 48, 64]);
        let mut f = Filter::new(4);
        for _ in 0..4 {
            f.push(0, 0);
        }
        assert_eq!([64, 0, 0, 0, 0].map(|v| f.push(0, v)), [16, 16, 16, 16, 0]);
        let mut f = Filter::new(5);
        assert_eq!([-1, 0, 1].map(|v| f.push(0, v)), [-1, -1, 0]);
    }
    #[test]
    fn changing_bandwidth_uses_prior_samples_and_axes_keep_distinct_apertures() {
        let mut f = Filter::new(6);
        for v in [4, 8, 12, 16] {
            f.push(0, v);
        }
        f.select(4);
        assert_eq!(f.push(0, 20), 14);
        assert_eq!(f.push(1, -3), -3);
        f.select(6);
        assert_eq!(f.push(0, 64), 64);
    }
}
