//! Asynchronous event counter and its PWM gate (REJ09B0152-0300 §13).
//!
//! Two eight-bit counters or one cascaded sixteen-bit counter share the actual
//! IRQAEC/IECPWM gating signal. Internal counting is analytical; external input
//! and PWM boundaries enter the same counter update. Controller requests are
//! edge notifications, not aliases of the OVH/OVL flags.
//!
//! The initial prescaler polarity, gate/clock coincidences, and interrupt
//! synchronizer delay are explicit reference-edge witnesses. The manual bounds
//! gate-induced error by one count and interrupt synchronization by one cycle;
//! this implementation does not claim sub-state silicon characterization.
use super::clocks::{Clocks, Tap};
use crate::{
    error::Error,
    time::{Time, TimeError},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aec {
    period: u16,
    duty: u16,
    edges: u8,
    clock: u8,
    status: u8,
    count: [u8; 2], // H, L
    seen: u8,
    pins: [Option<bool>; 3], // AEVH, AEVL, IRQAEC
    module: bool,
    system_running: bool,
    pwm_running: bool,
    pad_enabled: bool,
    last: [u64; 2],
    pwm_last: u64,
    pwm_phase: u16,
    pwm_high: bool,
    requests: u8, // 1=IRREC overflow, 2=IRREC2 gate edge, consumed by MCU.
    at: Time,
}
impl Default for Aec {
    fn default() -> Self {
        Self {
            period: 0xffff,
            duty: 0,
            edges: 0,
            clock: 0,
            status: 0,
            count: [0; 2],
            seen: 0,
            pins: [None; 3],
            module: false,
            system_running: false,
            pwm_running: false,
            pad_enabled: true,
            last: [0; 2],
            pwm_last: 0,
            pwm_phase: 0,
            pwm_high: false,
            requests: 0,
            at: Time::ZERO,
        }
    }
}
impl Aec {
    pub fn handles(a: u16) -> bool {
        matches!(a, 0xff92 | 0xff94..=0xff97)
    }
    pub fn pwm_enabled(&self) -> bool {
        self.edges & 2 != 0
    }
    pub fn pwm_uses_watch(&self) -> bool {
        self.clock & 14 == 12
    }
    pub fn pwm_output(&self) -> Option<bool> {
        (self.module && self.pwm_enabled() && self.pad_enabled).then_some(self.pwm_high)
    }
    fn source(&self, i: usize) -> u8 {
        self.clock >> if i == 0 { 6 } else { 4 } & 3
    }
    fn tap(&self, i: usize) -> Tap {
        Tap::system(1u32 << self.source(i))
    }
    fn pwm_tap(&self) -> Tap {
        if self.pwm_uses_watch() {
            Tap::watch(16)
        } else {
            Tap::system(2u32 << ((self.clock >> 1) & 7))
        }
    }
    fn independent(&self) -> bool {
        self.status & 16 != 0
    }
    fn enabled(&self, i: usize) -> bool {
        self.module && self.status & (8 >> i) != 0 && self.status & (2 >> i) != 0
    }
    fn gate(&self) -> bool {
        if self.pwm_enabled() {
            self.pwm_high
        } else {
            self.pins[2].unwrap_or(false)
        }
    }
    fn selected(mode: u8, old: bool, new: bool) -> bool {
        old != new
            && match mode {
                0 => !new,
                1 => new,
                2 => true,
                _ => false,
            }
    }
    fn edge_mode(&self, i: usize) -> u8 {
        self.edges >> if i == 0 { 6 } else { 4 } & 3
    }
    /// All increments, whether clock-derived or external, use this recurrence.
    fn increment(&mut self, i: usize, n: u64) {
        if n == 0 || !self.enabled(i) || (i == 0 && !self.independent()) {
            return;
        }
        let total = u128::from(self.count[i]) + u128::from(n);
        self.count[i] = total as u8;
        let wraps = (total >> 8) as u64;
        if self.independent() {
            if wraps != 0 {
                self.status |= 0x80 >> i;
                self.requests |= 1;
            }
        } else if i == 1 && self.enabled(0) && wraps != 0 {
            let high = u128::from(self.count[0]) + u128::from(wraps);
            self.count[0] = high as u8;
            if high > 255 {
                self.status |= 0x80;
                self.requests |= 1;
            }
        }
    }
    fn clocks_to(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        for i in 0..2 {
            let tick = c.ticks(now, self.tap(i));
            if self.system_running && self.gate() && self.source(i) != 0 {
                let n = tick.checked_sub(self.last[i]).ok_or(TimeError::Reversed)?;
                self.increment(i, n);
            }
            self.last[i] = tick;
        }
        Ok(())
    }
    /// Selected internal prescaler is high for the first half of its period.
    /// Only used when gate return itself can introduce a count (§13.4.5).
    fn source_level(&self, i: usize, now: Time, c: &Clocks) -> Option<bool> {
        if self.source(i) == 0 {
            self.pins[i]
        } else if self.system_running {
            Some(c.high(now, self.tap(i)))
        } else {
            None
        }
    }
    fn gate_transition(&mut self, old: bool, new: bool, now: Time, c: &Clocks) {
        if !self.module || old == new {
            return;
        }
        if Self::selected((self.edges >> 2) & 3, old, new) {
            self.requests |= 2;
        }
        for i in 0..2 {
            if self.source_level(i, now, c) == Some(true) {
                let mode = if self.source(i) == 0 {
                    self.edge_mode(i)
                } else {
                    1
                };
                if Self::selected(mode, old, new) {
                    self.increment(i, 1);
                }
            }
        }
    }
    fn pwm_distance(&self) -> u64 {
        if self.pwm_high {
            u64::from(self.period) - u64::from(self.pwm_phase) + 1
        } else {
            u64::from(self.duty) - u64::from(self.pwm_phase) + 1
        }
    }
    fn pwm_deadline(&self, c: &Clocks) -> Result<Option<Time>, Error> {
        if !self.module || !self.pwm_enabled() || !self.pwm_running || self.duty >= self.period {
            return Ok(None);
        }
        Ok(Some(
            c.edge(
                self.pwm_last
                    .checked_add(self.pwm_distance())
                    .ok_or(TimeError::Overflow)?,
                self.pwm_tap(),
            )?,
        ))
    }
    pub fn sync(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        if now < self.at {
            return Err(TimeError::Reversed.into());
        }
        if !self.module {
            // No AEC clock or output evolution in module standby. Rejoining
            // the shared phases is done once by set_power, not at every
            // unrelated MCU read. Pin baselines are still retained separately.
            self.at = now;
            return Ok(());
        }
        while let Some(due) = self.pwm_deadline(c)? {
            if due > now {
                break;
            }
            self.clocks_to(due, c)?;
            let k = self.pwm_distance();
            self.pwm_last = self.pwm_last.checked_add(k).ok_or(TimeError::Overflow)?;
            self.pwm_phase = if self.pwm_high {
                0
            } else {
                self.duty.wrapping_add(1)
            };
            let old = self.pwm_high;
            self.pwm_high = !old;
            self.gate_transition(old, self.pwm_high, due, c);
        }
        self.clocks_to(now, c)?;
        let tick = c.ticks(now, self.pwm_tap());
        if self.module && self.pwm_running && self.pwm_enabled() && self.duty < self.period {
            let n = tick.checked_sub(self.pwm_last).ok_or(TimeError::Reversed)?;
            self.pwm_phase = self.pwm_phase.wrapping_add(n as u16);
        }
        self.pwm_last = tick;
        self.at = now;
        Ok(())
    }
    /// Pin selection establishes a baseline. Ordinary changes subsequently
    /// traverse the same AND-gate semantics as PWM changes. This deliberately
    /// preserves the possible gate-return edge instead of dropping it.
    pub fn input_pins(
        &mut self,
        pins: [Option<bool>; 3],
        now: Time,
        c: &Clocks,
    ) -> Result<(), Error> {
        if now == self.at && pins == self.pins {
            return Ok(());
        }
        self.sync(now, c)?;
        let old_gate = self.gate();
        let old_pins = self.pins;
        self.pins = pins;
        let new_gate = self.gate();
        if self.module {
            if !self.pwm_enabled()
                && old_pins[2].is_some()
                && pins[2].is_some()
                && Self::selected((self.edges >> 2) & 3, old_gate, new_gate)
            {
                self.requests |= 2;
            }
            for i in 0..2 {
                if self.source(i) == 0 {
                    if let (Some(old), Some(new)) = (old_pins[i], pins[i]) {
                        if (self.pwm_enabled() || (old_pins[2].is_some() && pins[2].is_some()))
                            && Self::selected(self.edge_mode(i), old && old_gate, new && new_gate)
                        {
                            self.increment(i, 1);
                        }
                    }
                } else if old_gate != new_gate
                    && old_pins[2].is_some()
                    && pins[2].is_some()
                    && self.source_level(i, now, c) == Some(true)
                    && Self::selected(1, old_gate, new_gate)
                {
                    self.increment(i, 1);
                }
            }
        }
        Ok(())
    }
    pub fn set_power(
        &mut self,
        module: bool,
        system: bool,
        pwm_clock: bool,
        pad: bool,
        now: Time,
        c: &Clocks,
    ) -> Result<(), Error> {
        self.sync(now, c)?;
        self.module = module;
        self.system_running = system;
        self.pwm_running = pwm_clock;
        self.pad_enabled = pad;
        for i in 0..2 {
            self.last[i] = c.ticks(now, self.tap(i));
        }
        self.pwm_last = c.ticks(now, self.pwm_tap());
        Ok(())
    }
    pub fn take_requests(&mut self) -> u8 {
        let r = self.requests;
        self.requests = 0;
        r
    }
    pub fn read(&mut self, a: u16) -> u8 {
        if a == 0xff95 {
            self.seen = self.status & 0xc0;
        }
        self.peek(a)
    }
    pub fn peek(&self, a: u16) -> u8 {
        match a {
            0xff8c => (self.period >> 8) as u8,
            0xff8d => self.period as u8,
            0xff8e => (self.duty >> 8) as u8,
            0xff8f => self.duty as u8,
            0xff92 => self.edges,
            0xff94 => self.clock,
            0xff95 => self.status,
            0xff96 => self.count[0],
            0xff97 => self.count[1],
            _ => 0,
        }
    }
    pub fn read_word(&self, a: u16) -> Result<u16, Error> {
        if a == 0xff8c {
            Ok(self.period)
        } else {
            Err(Error::Unsupported {
                component: "AEC",
                detail: "ECPWDR has an undefined read value (§13.3.2)",
                address: a,
            })
        }
    }
    pub fn write_word(&mut self, a: u16, value: u16, now: Time, c: &Clocks) -> Result<(), Error> {
        self.sync(now, c)?;
        if self.pwm_enabled() {
            return Err(Error::Unsupported {
                component: "AEC",
                detail: "stop PWM before changing period or duty (§13.6)",
                address: a,
            });
        }
        match a {
            0xff8c => self.period = value,
            0xff8e => self.duty = value,
            _ => {
                return Err(Error::Unmapped {
                    address: a,
                    write: true,
                    width: 2,
                })
            }
        }
        Ok(())
    }
    pub fn write(&mut self, a: u16, value: u8, now: Time, c: &Clocks) -> Result<(), Error> {
        self.sync(now, c)?;
        match a {
            0xff92 => {
                if value & 1 != 0 || [2, 4, 6].into_iter().any(|n| value >> n & 3 == 3) {
                    return Err(Error::Unsupported {
                        component: "AEC",
                        detail: "prohibited edge selection or reserved bit",
                        address: a,
                    });
                }
                let was = self.pwm_enabled();
                self.edges = value;
                if was != self.pwm_enabled() {
                    self.pwm_phase = 0;
                    self.pwm_high = false;
                }
            }
            0xff94 => {
                if value & 1 != 0 || value & 14 == 14 {
                    return Err(Error::Unsupported {
                        component: "AEC",
                        detail: "prohibited PWM clock or reserved bit",
                        address: a,
                    });
                }
                if self.pwm_enabled() && (self.clock ^ value) & 14 != 0 {
                    return Err(Error::Unsupported {
                        component: "AEC",
                        detail: "stop PWM before changing PWCK (§13.3.4)",
                        address: a,
                    });
                }
                self.clock = value;
            }
            0xff95 => {
                if value & 0x20 != 0 {
                    return Err(Error::Unsupported {
                        component: "AEC",
                        detail: "reserved ECCSR bit",
                        address: a,
                    });
                }
                if !self.independent()
                    && self.status & 2 != 0
                    && value & 2 != 0
                    && (self.status ^ value) & 8 != 0
                {
                    return Err(Error::Unsupported {
                        component: "AEC",
                        detail:
                            "CUEH cannot change while 16-bit high counter is released (§13.6.3)",
                        address: a,
                    });
                }
                if value & 16 == 0 && value & 2 != 0 && value & 8 == 0 {
                    return Err(Error::Unsupported {
                        component: "AEC",
                        detail: "enable CUEH before releasing CRCH in 16-bit mode (§13.6.3)",
                        address: a,
                    });
                }
                self.status = (self.status & !(self.seen & !value) & 0xc0) | (value & 31);
                self.seen &= value;
                for i in 0..2 {
                    if value & (2 >> i) == 0 {
                        self.count[i] = 0;
                    }
                }
            }
            0xff96..=0xff97 => {} // Read-only counters: writes cannot preload them.
            _ => {
                return Err(Error::Unmapped {
                    address: a,
                    write: true,
                    width: 1,
                })
            }
        }
        for i in 0..2 {
            self.last[i] = c.ticks(now, self.tap(i));
        }
        self.pwm_last = c.ticks(now, self.pwm_tap());
        Ok(())
    }
    pub fn deadline(&self, c: &Clocks) -> Result<Option<Time>, Error> {
        let mut next = self.pwm_deadline(c)?;
        if self.gate() && self.system_running {
            for i in 0..2 {
                if self.source(i) == 0 || !self.enabled(i) || (i == 0 && !self.independent()) {
                    continue;
                }
                let distance = if self.independent() {
                    Some(256 - u64::from(self.count[i]))
                } else if i == 1 && self.enabled(0) {
                    Some(65536 - (u64::from(self.count[0]) * 256 + u64::from(self.count[1])))
                } else {
                    None
                };
                if let Some(n) = distance {
                    let due = c.edge(
                        self.last[i].checked_add(n).ok_or(TimeError::Overflow)?,
                        self.tap(i),
                    )?;
                    next = Some(next.map_or(due, |t| t.min(due)));
                }
            }
        }
        Ok(next)
    }
}
