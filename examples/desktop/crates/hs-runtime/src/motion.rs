use borsh::{BorshDeserialize, BorshSerialize};
use hs_core::Acceleration;

#[derive(Clone, BorshSerialize, BorshDeserialize)]
pub(crate) struct Motion {
    pub walking: bool,
    pub cadence: u16,
    phase: u64,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            walking: false,
            cadence: 120,
            phase: 0,
        }
    }
}

impl Motion {
    pub fn sample(&mut self) -> Acceleration {
        let mut acceleration = Acceleration::STILL;
        if self.walking {
            self.phase = (self.phase + u64::from(self.cadence)) % 120_000;
            acceleration.x = wave(self.phase) * 28 / 100;
            acceleration.y = wave(self.phase / 2) / 10;
            acceleration.z += wave(self.phase) * 85 / 100;
        }
        acceleration
    }
}

fn wave(phase: u64) -> i32 {
    const QUARTER: [i32; 17] = [
        0, 98017, 195090, 290285, 382683, 471397, 555570, 634393, 707107, 773010, 831470, 881921,
        923880, 956940, 980785, 995185, 1000000,
    ];
    let at = phase % 60000 * 64;
    let cell = (at / 60000) as usize;
    let sample = |cell: usize| match cell % 64 {
        cell @ 0..=16 => QUARTER[cell],
        cell @ 17..=32 => QUARTER[32 - cell],
        cell @ 33..=48 => -QUARTER[cell - 32],
        cell => -QUARTER[64 - cell],
    };
    let a = sample(cell);
    let b = sample(cell + 1);
    a + ((i64::from(b - a) * (at % 60000) as i64) / 60000) as i32
}
