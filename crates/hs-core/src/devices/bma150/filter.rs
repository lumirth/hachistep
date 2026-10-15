//! Power-of-two moving averages over actual ADC samples. Retaining 64 samples
//! allows a bandwidth change to select a new window without inventing history.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct Filter {
    history: [[i16; 64]; 3],
    next: [u8; 3],
    count: [u8; 3],
    #[borsh(skip)]
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

impl Filter {
    pub(super) fn rebuild(&mut self) -> Result<(), crate::Error> {
        crate::state::require(
            self.shift <= 6
                && self.next.iter().all(|n| *n < 64)
                && self.count.iter().all(|n| *n <= 64)
                && self
                    .history
                    .iter()
                    .flatten()
                    .all(|v| (-512..=511).contains(v)),
            "invalid sensor filter history",
        )?;
        for axis in 0..3 {
            self.sum[axis] = (0..self.window().min(usize::from(self.count[axis])))
                .map(|n| {
                    i32::from(self.history[axis][(usize::from(self.next[axis]) + 63 - n) & 63])
                })
                .sum();
        }
        Ok(())
    }
}
