//! Timer W: compare/PWM, paired buffers, capture, and external event counting.
//! Ordered conflicts follow REJ09B0152-0300 §10.7. The three-reference-edge
//! input pipeline follows Figs.10.15/10.17 at reference-edge granularity;
//! metastability, sub-state propagation and out-of-spec short pulses are not
//! characterized. There is one counter recurrence for internal/external clocks.
use super::clocks::{ClockWait, Clocks, Tap};
use crate::{
    error::Error,
    time::{Time, TimeError},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerW {
    mode: u8,
    control: u8,
    enable: u8,
    status: u8,
    io: [u8; 2],
    count: u16,
    general: [u16; 4],
    output: u8,
    seen: u8,
    last: u64,
    gate: bool,
    at: Time,
    clear_at: Option<Time>,
    capture_before: [u16; 4],
    capture_visible: [Option<ClockWait>; 4],
    pins: [Option<bool>; 5],
    pipeline: [[Option<bool>; 5]; 3],
    input_next: Option<ClockWait>,
}
impl Default for TimerW {
    fn default() -> Self {
        Self {
            mode: 0x48,
            control: 0,
            enable: 0x70,
            status: 0x70,
            io: [0x88; 2],
            count: 0,
            general: [0xffff; 4],
            output: 0,
            seen: 0,
            last: 0,
            gate: false,
            at: Time::ZERO,
            clear_at: None,
            capture_before: [0xffff; 4],
            capture_visible: [None; 4],
            pins: [None; 5],
            pipeline: [[None; 5]; 3],
            input_next: None,
        }
    }
}
impl TimerW {
    pub fn uses_watch(&self) -> bool {
        matches!(self.control >> 4 & 7, 4..=6)
    }
    pub fn uses_external(&self) -> bool {
        self.control & 0x70 == 0x70
    }
    fn tap(&self) -> Tap {
        match self.control >> 4 & 7 {
            0 => Tap::system(1),
            1 => Tap::system(2),
            2 => Tap::system(4),
            3 => Tap::system(8),
            4 => Tap::watch(1),
            5 => Tap::watch(4),
            6 => Tap::watch(16),
            _ => Tap::system(1), // No counter appointment in external mode.
        }
    }
    fn running(&self) -> bool {
        self.gate && self.mode & 0x80 != 0
    }
    fn pwm(&self, i: usize) -> bool {
        i > 0 && self.mode & (1 << (i - 1)) != 0
    }
    fn io_mode(&self, i: usize) -> u8 {
        self.io[i / 2] >> ((i % 2) * 4) & 7
    }
    fn compare(&self, i: usize) -> bool {
        self.io_mode(i) & 4 == 0 || self.pwm(i) || (i == 0 && self.mode & 7 != 0)
    }
    pub fn captures(&self, i: usize) -> bool {
        i < 4 && !self.compare(i) && !(i >= 2 && self.mode & (0x10 << (i - 2)) != 0)
    }
    fn distance(&self) -> u64 {
        let mut d = 65536 - u64::from(self.count);
        for (i, value) in self.general.into_iter().enumerate() {
            if self.compare(i) {
                d = d.min(u64::from(value.wrapping_sub(self.count)) + 1);
            }
        }
        d
    }
    /// One canonical counter transition, including the old-register snapshot
    /// used by *all* comparisons and paired-buffer transfers at this edge.
    fn counter_step(&mut self, edges: u64, at: Time) {
        let before = self.count;
        let regs = self.general;
        self.count = self.count.wrapping_add(edges as u16);
        let mut matches = 0u8;
        for (i, value) in regs.into_iter().enumerate() {
            if self.compare(i) && u64::from(value.wrapping_sub(before)) + 1 == edges {
                matches |= 1 << i;
            }
        }
        if 65536 - u64::from(before) == edges {
            self.status |= 0x80;
        }
        self.status |= matches;
        for i in 0..4 {
            let hit = matches & (1 << i) != 0;
            if self.pwm(i) {
                let period = matches & 1 != 0;
                // §10.4.2: simultaneous duty/period match does NOT change the
                // output; it does not unconditionally restore initial polarity.
                if hit != period {
                    self.set_output(i, (self.control & (1 << i) != 0) == period);
                }
            } else if hit {
                match self.io_mode(i) {
                    1 => self.set_output(i, false),
                    2 => self.set_output(i, true),
                    3 => self.output ^= 1 << i,
                    _ => {}
                }
            }
        }
        if matches & 1 != 0 && self.control & 0x80 != 0 {
            self.count = 0;
            self.clear_at = Some(at);
        }
        for i in 0..2 {
            if self.mode & (0x10 << i) != 0 && matches & (1 << i) != 0 {
                self.general[i] = regs[i + 2];
            }
        }
    }
    fn clock_to(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        let tick = clocks.ticks(now, self.tap());
        if self.running() && !self.uses_external() {
            let mut n = tick.checked_sub(self.last).ok_or(TimeError::Reversed)?;
            let mut ordinal = self.last;
            while n != 0 {
                let k = n.min(self.distance());
                ordinal = ordinal.checked_add(k).ok_or(TimeError::Overflow)?;
                let at = clocks.edge(ordinal, self.tap())?;
                self.counter_step(k, at);
                n -= k;
            }
        }
        self.last = tick;
        Ok(())
    }
    pub fn sync(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if now < self.at {
            return Err(TimeError::Reversed.into());
        }
        while let Some(due) = self
            .input_next
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(clocks))?
        {
            if due > now {
                break;
            }
            self.clock_to(Time::from_raw(due.raw().saturating_sub(1)), clocks)?;
            let captured_count = self.count;
            let old_regs = self.general;
            self.clock_to(due, clocks)?;
            let old = self.pipeline[2];
            let new = self.pipeline[1];
            self.pipeline[2] = self.pipeline[1];
            self.pipeline[1] = self.pipeline[0];
            self.pipeline[0] = self.pins;
            if self.running()
                && self.uses_external()
                && old[4] == Some(false)
                && new[4] == Some(true)
            {
                self.counter_step(1, due);
            }
            if self.gate {
                for i in 0..4 {
                    if !self.captures(i) {
                        continue;
                    }
                    let transition = matches!((old[i],new[i]),(Some(a),Some(b)) if a != b);
                    let valid = match self.io_mode(i) & 3 {
                        0 => new[i] == Some(true),
                        1 => new[i] == Some(false),
                        _ => true,
                    };
                    if transition && valid {
                        self.capture_before[i] = old_regs[i];
                        self.capture_visible[i] =
                            Some(ClockWait::after(due, 1, Tap::cpu(), clocks)?);
                        self.general[i] = captured_count;
                        self.status |= 1 << i;
                        if i < 2 && self.mode & (0x10 << i) != 0 {
                            self.capture_before[i + 2] = old_regs[i + 2];
                            self.capture_visible[i + 2] =
                                Some(ClockWait::after(due, 1, Tap::cpu(), clocks)?);
                            self.general[i + 2] = old_regs[i];
                        }
                    }
                }
            }
            self.input_next = None;
            self.schedule_input(due, clocks)?;
        }
        self.clock_to(now, clocks)?;
        // Retire visibility waits under their old clock epoch. Retaining an
        // already-consumed edge target across a later clock switch would ask
        // the new clock epoch to project a target in its past.
        for visible in &mut self.capture_visible {
            if visible
                .as_ref()
                .map_or(Ok(None), |w| w.deadline(clocks))?
                .is_some_and(|at| at <= now)
            {
                *visible = None;
            }
        }
        self.at = now;
        Ok(())
    }
    fn schedule_input(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.gate && self.input_next.is_none() && self.pipeline.iter().any(|s| *s != self.pins) {
            self.input_next = Some(ClockWait::after(now, 1, Tap::cpu(), clocks)?);
        }
        Ok(())
    }
    /// Actual selected package pins A,B,C,D,FTCI. Unselected inputs are None.
    /// Selecting an input establishes its baseline; it does not invent an edge.
    pub fn input_pins(
        &mut self,
        mut pins: [Option<bool>; 5],
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        self.sync(now, clocks)?;
        for (i, pin) in pins.iter_mut().enumerate() {
            if (i < 4 && !self.captures(i)) || (i == 4 && !self.uses_external()) {
                *pin = None;
            }
            if pin.is_none() || self.pins[i].is_none() {
                for stage in &mut self.pipeline {
                    stage[i] = *pin;
                }
            }
        }
        self.pins = pins;
        self.schedule_input(now, clocks)
    }
    fn set_output(&mut self, i: usize, high: bool) {
        if high {
            self.output |= 1 << i;
        } else {
            self.output &= !(1 << i);
        }
    }
    pub fn outputs(&self) -> u8 {
        self.output
    }
    pub fn drives(&self) -> u8 {
        (0..4).fold(0, |m, i| {
            m | (u8::from(matches!(self.io_mode(i), 1..=3) || self.pwm(i)) << i)
        })
    }
    pub fn interrupt(&self) -> bool {
        self.interrupt_with_enable(0)
    }
    pub(crate) fn interrupt_with_enable(&self, retained: u8) -> bool {
        self.status & (self.enable | retained) & 0x8f != 0
    }
    pub fn read(&mut self, address: u16) -> u8 {
        if address == 0xf0f3 {
            self.seen = self.status & 0x8f;
        }
        self.peek(address)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0xf0f0 => self.mode,
            0xf0f1 => self.control,
            0xf0f2 => self.enable,
            0xf0f3 => self.status,
            0xf0f4..=0xf0f5 => self.io[usize::from(address - 0xf0f4)],
            0xf0f6..=0xf0ff => {
                let v = self.word(address & !1);
                if address & 1 == 0 {
                    (v >> 8) as u8
                } else {
                    v as u8
                }
            }
            _ => 0,
        }
    }
    pub fn word(&self, address: u16) -> u16 {
        match address {
            0xf0f6 => self.count,
            0xf0f8 | 0xf0fa | 0xf0fc | 0xf0fe => self.general[usize::from((address - 0xf0f8) / 2)],
            _ => 0,
        }
    }
    pub fn read_word(&self, address: u16, clocks: &Clocks) -> Result<u16, Error> {
        if matches!(address, 0xf0f8 | 0xf0fa | 0xf0fc | 0xf0fe) {
            let i = usize::from((address - 0xf0f8) / 2);
            if let Some(wait) = self.capture_visible[i] {
                if wait.deadline(clocks)?.is_some_and(|ready| self.at < ready) {
                    return Ok(self.capture_before[i]);
                }
            }
        }
        Ok(self.word(address))
    }
    pub fn write_word(
        &mut self,
        address: u16,
        value: u16,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        self.sync(now, clocks)?;
        match address {
            0xf0f6 => {
                if self.clear_at != Some(now) {
                    self.count = value;
                }
            } // Clear wins over TCNT write (§10.7.2).
            0xf0f8 | 0xf0fa | 0xf0fc | 0xf0fe => {
                let i = usize::from((address - 0xf0f8) / 2);
                self.general[i] = value; // CPU write wins over capture/buffer transfer.
                self.capture_visible[i] = None;
            }
            _ => {
                return Err(Error::Unmapped {
                    address,
                    write: true,
                    width: 2,
                })
            }
        }
        Ok(())
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        self.sync(now, clocks)?;
        let old_clock = self.running() && !self.uses_external() && clocks.high(now, self.tap());
        let old_internal = !self.uses_external();
        let old_selection = self.control & 0x70;
        match address {
            0xf0f0 => self.mode = value | 0x48,
            0xf0f1 => {
                self.control = value;
                self.output = value & 15;
            } // Output level settings apply immediately (§10.3.2).
            0xf0f2 => self.enable = value | 0x70,
            0xf0f3 => {
                if self.gate {
                    self.status &= !(self.seen & !value);
                    self.seen &= value;
                    self.status |= 0x70;
                }
            } // §10.7.4: pending status cannot be cleared in module standby.
            0xf0f4..=0xf0f5 => self.io[usize::from(address - 0xf0f4)] = value | 0x88,
            _ => {
                return Err(Error::Unsupported {
                    component: "Timer W",
                    detail: "TCNT and GR registers require 16-bit accesses (§10.3.7–8)",
                    address,
                })
            }
        }
        // §10.7: switching internal inputs from low to high is itself an
        // increment pulse. Reset/gating and external synchronization are
        // separate circuits; they do not pass through this selector write.
        if address == 0xf0f1
            && old_selection != self.control & 0x70
            && old_internal
            && !self.uses_external()
            && self.running()
            && !old_clock
            && clocks.high(now, self.tap())
        {
            self.counter_step(1, now);
        }
        self.last = clocks.ticks(now, self.tap());
        Ok(())
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.gate == gate {
            return Ok(());
        }
        self.sync(now, clocks)?;
        self.gate = gate;
        self.last = clocks.ticks(now, self.tap());
        if let Some(wait) = &mut self.input_next {
            if gate {
                wait.resume(now, clocks)?;
            } else {
                wait.pause(now, clocks)?;
            }
        }
        self.schedule_input(now, clocks)
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        let count = if self.running() && !self.uses_external() && clocks.available(self.tap()) {
            Some(
                clocks.edge(
                    self.last
                        .checked_add(self.distance())
                        .ok_or(TimeError::Overflow)?,
                    self.tap(),
                )?,
            )
        } else {
            None
        };
        let input = self
            .input_next
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(clocks))?;
        Ok(count.into_iter().chain(input).min())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compare_occurs_on_leaving_the_matching_count() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut w = TimerW::default();
        w.set_gate(true, Time::ZERO, &c).unwrap();
        w.write_word(0xf0f8, 2, Time::ZERO, &c).unwrap();
        w.write(0xf0f1, 0x80, Time::ZERO, &c).unwrap();
        w.write(0xf0f0, 0x80, Time::ZERO, &c).unwrap();
        w.sync(c.edge(2, Tap::system(1)).unwrap(), &c).unwrap();
        assert_eq!(w.count, 2);
        assert_eq!(w.status & 1, 0);
        w.sync(c.edge(3, Tap::system(1)).unwrap(), &c).unwrap();
        assert_eq!(w.count, 0);
        assert_eq!(w.status & 1, 1);
    }
}
