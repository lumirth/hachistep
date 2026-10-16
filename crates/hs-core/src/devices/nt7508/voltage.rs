//! NT7508 V1.0 pp.25-28,42-44,48-50: nominal steady-state supplies.
use super::Nt7508;

/// Microvolts relative to VSS2. Missing levels are not internally driven within
/// the documented operating conditions. Capacitor transients and glass response
/// are separate from these steady-state values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LcdVoltages {
    pub pump: Option<u32>,
    pub target: u32,
    /// V0 through V4. The board has no external source for these pins.
    pub levels: [Option<u32>; 5],
}
impl LcdVoltages {
    pub const OFF: Self = Self {
        pump: None,
        target: 0,
        levels: [None; 5],
    };
}
impl Nt7508 {
    pub fn voltages(&self) -> LcdVoltages {
        // VREF=1.4 V at 25 C. Keep the temperature and EV arithmetic rational
        // until the final rounding, so native and WASM produce identical rails.
        let coefficient = if self.temperature_slope == 0 { 125 } else { 50 };
        let temperature =
            100_000_000i64 - (i64::from(self.temperature_millicelsius) - 25_000) * coefficient;
        // F3's signed four-bit offset is accepted in OTP test mode. OTP_PWR
        // is unconnected on the board, so there is no host-side fuse operation.
        let trim = if self.otp_control & 1 != 0 {
            i64::from((self.contrast_trim as i8) << 4 >> 4)
        } else {
            0
        };
        let ev = 147 + i64::from(self.contrast) + trim;
        let gain = [345, 450, 555, 660, 765, 870, 975, 1080][usize::from(self.regulator)];
        let numerator = 1_400_000i128 * i128::from(temperature.max(0)) * i128::from(ev) * gain;
        let denominator = 100_000_000i128 * 210 * 100;
        let target = ((numerator + denominator / 2) / denominator).min(i128::from(u32::MAX)) as u32;
        let analog = self.supplied
            && (2400..=3600).contains(&self.supply_millivolts)
            && (-30_000..=85_000).contains(&self.temperature_millicelsius)
            && !self.power_save
            && self.ready.is_none();
        let multiple = (3 + (self.booster & 3) + ((self.booster & 8) >> 1)).min(8);
        let pump = (analog && self.power_control & 4 != 0)
            .then_some(u32::from(self.supply_millivolts) * 1000 * u32::from(multiple));
        let mut levels = [None; 5];
        // p.25 requires pump > compensated VLCD + 3 V. Outside that envelope
        // the datasheet supplies no dropout law; do not invent a regulated V0.
        if self.power_control & 2 != 0
            && (4_000_000..=15_000_000).contains(&target)
            && pump.is_some_and(|v| u64::from(v) > u64::from(target) + 3_000_000)
        {
            levels[0] = Some(target);
            if self.power_control & 1 != 0 {
                let n = u32::from(self.bias) + 5;
                for (level, factor) in levels[1..].iter_mut().zip([n - 1, n - 2, 2, 1]) {
                    *level = Some(
                        ((u64::from(target) * u64::from(factor) + u64::from(n / 2)) / u64::from(n))
                            as u32,
                    );
                }
            }
        }
        LcdVoltages {
            pump,
            target,
            levels,
        }
    }
}
