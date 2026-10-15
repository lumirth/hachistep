//! Bosch §3.2: hysteretic criteria, millisecond debounce, and motion history.
//! Register values remain owned by the sensor; alert adjusts working durations.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
struct Threshold {
    axes: [bool; 3],
    count: u16,
    level: bool,
    latched: bool,
}
impl Threshold {
    fn active(&self, high: bool) -> bool {
        if high {
            self.axes.into_iter().any(|v| v)
        } else {
            self.axes.into_iter().all(|v| v)
        }
    }
    fn observe(&mut self, axis: usize, code: i16, threshold: u8, hysteresis: u8, high: bool) {
        let magnitude = i32::from(code).abs() * 255;
        let threshold = i32::from(threshold) * 512;
        let hysteresis = i32::from(hysteresis) * 32 * 512;
        if if high {
            magnitude >= threshold
        } else {
            magnitude <= threshold
        } {
            self.axes[axis] = true;
        } else if if high {
            magnitude < threshold - hysteresis
        } else {
            magnitude > threshold + hysteresis
        } {
            self.axes[axis] = false;
        }
        if !self.active(high) {
            self.level = false;
        }
    }
    fn tick(&mut self, high: bool, enabled: bool, duration: u8, mode: u8) -> bool {
        if !enabled {
            self.count = 0;
            self.level = false;
        } else if self.active(high) {
            self.count += 1;
            if self.count > u16::from(duration) {
                self.count = 0;
                self.level = true;
                self.latched = true;
                return true;
            }
        } else {
            self.count = if mode == 0 {
                0
            } else {
                self.count.saturating_sub(u16::from(mode))
            };
        }
        false
    }
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Interrupts {
    thresholds: [Threshold; 2], // Low-g, high-g.
    history: [[i16; 3]; 3],
    cursor: u8,
    observations: u8,
    divider: u8,
    motion_set: u8,
    motion_clear: u8,
    motion_active: bool,
    motion_level: bool,
    motion_latched: bool,
    alert: bool,
    durations: [u8; 2],
}
impl Interrupts {
    pub(super) fn configure(&mut self, registers: &[u8]) {
        if !self.alert {
            self.durations = [registers[0x0d], registers[0x0f]];
        }
    }
    pub(super) fn restart_acquisition(&mut self) {
        self.observations = 0;
        self.divider = 0;
        self.motion_set = 0;
        self.motion_clear = 0;
        self.motion_active = false;
        for t in &mut self.thresholds {
            t.count = 0;
            t.axes = [false; 3];
        }
    }
    pub(super) fn reset(&mut self, registers: &[u8]) {
        for t in &mut self.thresholds {
            t.count = 0;
            t.level = false;
            t.latched = false;
        }
        self.motion_level = false;
        self.motion_latched = false;
        self.motion_set = 0;
        self.motion_clear = 0;
        self.alert = false;
        self.configure(registers);
    }
    pub(super) fn axis(&mut self, axis: usize, code: i16, registers: &[u8]) {
        for (i, t) in self.thresholds.iter_mut().enumerate() {
            t.observe(
                axis,
                code,
                registers[0x0c + 2 * i],
                (registers[0x11] >> (3 * i)) & 7,
                i != 0,
            );
        }
    }
    pub(super) fn cycle(&mut self, codes: [i16; 3], window: usize, registers: &[u8]) {
        self.divider += 1;
        if usize::from(self.divider) < window {
            return;
        }
        self.divider = 0;
        let previous = self.history[usize::from(self.cursor)];
        self.history[usize::from(self.cursor)] = codes;
        self.cursor = (self.cursor + 1) % 3;
        self.observations = (self.observations + 1).min(4);
        if self.observations < 4 {
            return;
        }
        self.motion_active = (0..3).any(|i| {
            (i32::from(codes[i]) - i32::from(previous[i])).abs() >= i32::from(registers[0x10]) * 4
        });
        if self.motion_active {
            self.motion_set = self.motion_set.saturating_add(1);
            self.motion_clear = 0;
        } else {
            self.motion_clear = self.motion_clear.saturating_add(1);
            self.motion_set = 0;
        }
        let need = [1, 3, 5, 7][usize::from(registers[0x11] >> 6)];
        if self.motion_clear >= need {
            self.motion_level = false;
        }
        if self.motion_set >= need && registers[0x15] & 0x40 != 0 {
            // Alert takes precedence in the forbidden both-enabled encoding.
            if registers[0x0b] & 0x80 != 0 {
                self.alert = true;
            } else if registers[0x0b] & 0x40 != 0 {
                self.motion_level = true;
                self.motion_latched = true;
            }
        }
    }
    pub(super) fn millisecond(&mut self, registers: &[u8]) {
        if self.alert {
            self.durations = self.durations.map(|v| v.saturating_sub(1));
        }
        let mut triggered = false;
        for (i, t) in self.thresholds.iter_mut().enumerate() {
            triggered |= t.tick(
                i != 0,
                registers[0x0b] & (1 << i) != 0,
                self.durations[i],
                registers[0x0b] >> (2 + 2 * i) & 3,
            );
        }
        if triggered || (self.alert && self.durations == [0, 0]) {
            self.alert = false;
            self.configure(registers);
        }
    }
    pub(super) fn status(&self) -> u8 {
        (u8::from(self.thresholds[0].level) << 1)
            | u8::from(self.thresholds[1].level)
            | (u8::from(self.thresholds[0].latched) << 3)
            | (u8::from(self.thresholds[1].latched) << 2)
            | (u8::from(self.alert) << 4)
    }
    pub(super) fn output(&self, registers: &[u8]) -> bool {
        let latch = registers[0x15] & 0x10 != 0;
        self.thresholds.iter().enumerate().any(|(i, t)| {
            registers[0x0b] & (1 << i) != 0 && if latch { t.latched } else { t.level }
        }) || (registers[0x15] & 0x40 != 0
            && registers[0x0b] & 0xc0 == 0x40
            && if latch {
                self.motion_latched
            } else {
                self.motion_level
            })
    }
    /// Keep the sensor awake until a candidate interrupt meets its required
    /// duration or sample history, or stops qualifying.
    pub(super) fn verifying(&self, registers: &[u8]) -> bool {
        self.thresholds
            .iter()
            .enumerate()
            .any(|(i, t)| registers[0x0b] & (1 << i) != 0 && t.active(i != 0))
            || (registers[0x15] & 0x40 != 0
                && registers[0x0b] & 0xc0 != 0
                && (self.observations < 4 || self.motion_active))
            || self.alert
    }
}

impl Interrupts {
    pub(super) fn validate(&self) -> Result<(), crate::Error> {
        crate::state::require(
            self.thresholds.iter().all(|t| t.count <= 255)
                && self.cursor < 3
                && self.observations <= 4
                && self.divider < 64
                && self
                    .history
                    .iter()
                    .flatten()
                    .all(|v| (-512..=511).contains(v)),
            "invalid sensor interrupt history",
        )
    }
}
