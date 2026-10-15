//! WDT counter, private ROSC divider, qualified control writes, and reset cause.
//! Reset assertion is returned to the machine, which owns the 512-ROSC hold.
use super::bus::Recurrence;
use super::clocks::{Clocks, Source, Tap};
use crate::{cpu::WriteOrigin, error::Error, time::Time};

/// The private ROSC divider: while it counts (`rosc_required`), `rosc_last`
/// is the on-chip ordinal its current 2048-cycle period began at and
/// `rosc_phase` is zero; while it does not, `rosc_phase` retains the cycles
/// counted into that period and `rosc_last` is where counting stopped.
/// Synchronizing within a period therefore changes nothing.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Watchdog {
    mode: u8,
    control1: u8,
    control2: u8,
    count: u8,
    last: u64,
    rosc_last: u64,
    rosc_ticks: u64,
    rosc_phase: u16,
    seen_overflow: bool,
    gate: bool,
    source_available: bool,
    reset_held: bool,
    #[borsh(skip)]
    period_end: PeriodEnd,
}
/// When the divider's current period ends, under one clock revision; not
/// state.
#[derive(Clone, Debug, Default)]
struct PeriodEnd(core::cell::Cell<Option<(u64, u64, Time)>>);
impl PartialEq for PeriodEnd {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for PeriodEnd {}
impl Default for Watchdog {
    fn default() -> Self {
        Self {
            mode: 0xf0,
            control1: 0xae,
            control2: 0x57,
            count: 0,
            last: 0,
            rosc_last: 0,
            rosc_ticks: 0,
            rosc_phase: 0,
            seen_overflow: false,
            gate: true,
            source_available: true,
            reset_held: false,
            period_end: PeriodEnd::default(),
        }
    }
}
impl Watchdog {
    pub fn source(&self) -> Source {
        match self.mode & 15 {
            4 | 5 => Source::Watch,
            8..=15 => Source::System,
            _ => Source::OnChip,
        }
    }
    fn tap(&self) -> Option<Tap> {
        match self.mode & 15 {
            0..=3 => Some(Tap::on_chip(2048)),
            4 => Some(Tap::watch(16)),
            5 => Some(Tap::watch(256)),
            6 | 7 => None,
            n => Some(Tap::system(64 << (n - 8))),
        }
    }
    fn running(&self) -> bool {
        self.control1 & 4 != 0 && self.source_available && !self.reset_held && self.tap().is_some()
    }
    /// The control fields that clock gating reads: the source selection and
    /// the WDON bit that keeps the private oscillator running.
    pub(crate) fn gating(&self) -> (u8, u8) {
        (self.mode & 15, self.control1 & 4)
    }
    pub fn rosc_required(&self) -> bool {
        !self.reset_held && self.rosc_for_module(self.gate)
    }
    pub fn rosc_for_module(&self, gate: bool) -> bool {
        self.mode & 15 <= 3 && (gate || self.control1 & 4 != 0)
    }
    fn tick(&self, now: Time, clocks: &Clocks) -> u64 {
        if self.mode & 15 <= 3 {
            self.rosc_ticks
        } else {
            self.tap().map_or(0, |tap| clocks.ticks(now, tap))
        }
    }
    /// Whether the divider's current period has not ended by `now`.
    fn within_period(&self, now: Time, clocks: &Clocks) -> bool {
        let revision = clocks.revision();
        let end = match self.period_end.0.get() {
            Some((r, start, at)) if r == revision && start == self.rosc_last => at,
            _ => match clocks.edge(self.rosc_last + 2048, Tap::on_chip(1)) {
                Ok(at) => {
                    self.period_end.0.set(Some((revision, self.rosc_last, at)));
                    at
                }
                Err(_) => return false,
            },
        };
        now < end
    }
    /// Count the divider's completed periods through on-chip ordinal `raw`.
    fn rosc_settle(&mut self, raw: u64) {
        if self.rosc_required() {
            let periods = raw.saturating_sub(self.rosc_last) / 2048;
            self.rosc_ticks += periods;
            self.rosc_last += periods * 2048;
        }
    }
    /// After the divider's counting condition may have changed at `now`.
    fn rosc_transition(&mut self, counted: bool, now: Time, clocks: &Clocks) {
        let counting = self.rosc_required();
        if counted == counting {
            return;
        }
        let raw = clocks.ticks(now, Tap::on_chip(1));
        if counted {
            self.rosc_phase = (raw - self.rosc_last) as u16;
            self.rosc_last = raw;
        } else {
            self.rosc_last = raw - u64::from(self.rosc_phase);
            self.rosc_phase = 0;
        }
    }
    pub fn sync(&mut self, now: Time, clocks: &Clocks) -> bool {
        if self.mode & 15 <= 3 && self.rosc_required() && !self.within_period(now, clocks) {
            self.rosc_settle(clocks.ticks(now, Tap::on_chip(1)));
        }
        let tick = self.tick(now, clocks);
        let n = tick.saturating_sub(self.last);
        self.last = tick;
        if !self.running() || n == 0 {
            return false;
        }
        let overflow = n >= 256 - u64::from(self.count);
        self.count = self.count.wrapping_add(n as u8);
        if overflow {
            self.control2 |= 0x80;
            if self.control2 & 0x20 == 0 {
                self.control1 |= 1;
                return true;
            }
        }
        false
    }
    pub fn read(&mut self, address: u16) -> u8 {
        if address == 0xffb2 {
            self.seen_overflow = self.control2 & 0x80 != 0;
        }
        self.peek(address)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0xffb0 => self.mode,
            0xffb1 => self.control1,
            0xffb2 => self.control2,
            _ => self.count,
        }
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        origin: WriteOrigin,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        let counted = self.rosc_required();
        match address {
            0xffb0 => self.mode = value | 0xf0,
            0xffb1 if origin.is_mov() => self.control1 = self.control1_after(value),
            0xffb2 if origin.is_mov() => {
                let old = self.control2;
                let missed_clear =
                    old & 0x20 != 0 && origin == WriteOrigin::MovByteAbs8 { pc_bit1: false };
                if value & 0x80 == 0 && self.seen_overflow {
                    self.control2 &= !0x80;
                    self.seen_overflow = false;
                }
                for (inhibit, bit) in [(0x40, 0x20), (0x10, 8)] {
                    if value & inhibit == 0 && !(missed_clear && old & bit != 0 && value & bit == 0)
                    {
                        self.control2 = (self.control2 & !bit) | (value & bit);
                    }
                }
                self.control2 |= 0x57;
            }
            0xffb3 if self.control1 & 0x40 != 0 => self.count = value,
            _ => {}
        }
        self.rosc_transition(counted, now, clocks);
        self.last = self.tick(now, clocks);
        Ok(())
    }
    /// TCSRWD1 after a MOV of `value`: each field is written only when its
    /// write-inhibit bit is clear, and WDON and the reset flag only while
    /// TCSRWE was already set.
    fn control1_after(&self, value: u8) -> u8 {
        let old = self.control1;
        let mut control1 = old;
        if value & 0x80 == 0 {
            control1 = (control1 & !0x40) | (value & 0x40);
        }
        if value & 0x20 == 0 {
            control1 = (control1 & !0x10) | (value & 0x10);
        }
        if old & 0x10 != 0 {
            if value & 8 == 0 {
                control1 = (control1 & !4) | (value & 4);
            }
            if value & 3 == 0 {
                control1 &= !1;
            }
        }
        control1 | 0xaa
    }
    /// Whether the write changes no field, at whatever time it is written:
    /// the controls change only by writes and at overflow, and without a
    /// changed field a write only re-anchors the count, which
    /// synchronization does anyway.
    pub(crate) fn inert_write(&self, address: u16, value: u8, origin: WriteOrigin) -> bool {
        match address {
            0xffb0 => value | 0xf0 == self.mode,
            0xffb1 => !origin.is_mov() || self.control1_after(value) == self.control1,
            0xffb2 => !origin.is_mov(),
            _ => self.control1 & 0x40 == 0,
        }
    }
    pub fn set_power(
        &mut self,
        gate: bool,
        source_available: bool,
        reset_held: bool,
        now: Time,
        clocks: &Clocks,
    ) {
        let counted = self.rosc_required();
        self.gate = gate;
        self.source_available = source_available;
        self.reset_held = reset_held;
        self.rosc_transition(counted, now, clocks);
        self.last = self.tick(now, clocks);
    }
    pub fn reset(&mut self, watchdog: bool, now: Time, clocks: &Clocks) {
        *self = Self::default();
        if watchdog {
            self.control1 |= 1;
        }
        self.rosc_last = clocks.ticks(now, Tap::on_chip(1));
    }
    pub fn interrupt(&self) -> bool {
        self.interrupt_with_enable(0)
    }
    pub(crate) fn interrupt_with_enable(&self, retained: u8) -> bool {
        !self.reset_held && self.control2 & 0xa0 == 0xa0 && (self.control2 | retained) & 8 != 0
    }
    /// Control writes act on retained fields that change only at overflow,
    /// and re-anchor the already synchronized count. A counter write
    /// replaces the count, which matters elsewhere only at overflow.
    pub(crate) fn recurrence(address: u16) -> Recurrence {
        if address == 0xffb3 {
            Recurrence::Replaces
        } else {
            Recurrence::Settled
        }
    }
    /// TCSRWD1 can change retained controls locally when WDON holds. These
    /// fields do not affect counter evolution, so its lazy clock phase holds.
    /// The caller's interval already precedes every overflow consequence.
    /// A WDON change requires the board's clock-gating settlement instead.
    pub(crate) fn write_control1_local(&mut self, value: u8, origin: WriteOrigin) -> Option<bool> {
        let next = if origin.is_mov() {
            self.control1_after(value)
        } else {
            self.control1
        };
        if (next ^ self.control1) & 4 != 0 {
            return None;
        }
        let changed = next != self.control1;
        self.control1 = next;
        Some(changed)
    }
    /// A downward counter replacement on a shared clock, before the interval's
    /// next overflow consequence. It moves overflow later without changing
    /// flags; the new anchor discards earlier increments as `sync` then `write`
    /// would. The private oscillator's divider must settle through `sync`.
    pub(crate) fn replace_count_local(&mut self, value: u8, now: Time, clocks: &Clocks) -> bool {
        if self.mode & 15 <= 3 || self.control1 & 0x40 == 0 || value > self.count {
            return false;
        }
        let tick = self.tick(now, clocks);
        self.count = value;
        self.last = tick;
        true
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.running() || self.control2 & 0xa0 == 0xa0 {
            return Ok(None);
        }
        let Some(tap) = self.tap() else {
            return Ok(None);
        };
        let count = 256 - u64::from(self.count);
        let at = if self.mode & 15 <= 3 {
            clocks.edge(self.rosc_last + count * 2048, Tap::on_chip(1))?
        } else {
            clocks.edge(self.last + count, tap)?
        };
        Ok(Some(at))
    }
}

impl Watchdog {
    pub(crate) fn validate(&self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        let raw = clocks.ticks(now, Tap::on_chip(1));
        crate::state::require(
            self.rosc_phase < 2048
                && (!self.rosc_required() || self.rosc_phase == 0)
                && self.control1 & 0xaa == 0xaa
                && self.control2 & 0x57 == 0x57
                && self.mode & 0xf0 == 0xf0
                // The private divider can count only source edges that have
                // actually occurred, including across stopped clock intervals.
                && self.rosc_last <= raw
                && self.rosc_ticks <= self.rosc_last / 2048
                && raw
                    .saturating_sub(self.rosc_last)
                    .checked_add(u64::from(self.rosc_phase))
                    .and_then(|n| self.rosc_ticks.checked_add(n / 2048))
                    .is_some()
                && self.rosc_last.checked_add(524288).is_some()
                && self.last.checked_add(256).is_some()
                && self.rosc_ticks.checked_add(65536).is_some(),
            "invalid watchdog divider",
        )
    }
}
