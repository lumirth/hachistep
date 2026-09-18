//! MCU operating modes, clock/module controls, and external interrupt latches.
//! Invalid mode combinations stop explicitly; there is no product-level wake
//! shortcut based on a button name or retail firmware address.
use super::clocks::{ClockWait, Clocks, Source, Tap};
use crate::{error::Error, time::Time};
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum Mode {
    Active = 0,
    Subactive = 1,
    Sleep = 2,
    Subsleep = 3,
    Watch = 4,
    Standby = 5,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub sys1: u8,
    pub sys2: u8,
    pub osc: u8,
    feedback_cut: bool,
    pub gate1: u8,
    pub gate2: u8,
    pub iegr: u8,
    pub ien1: u8,
    pub ien2: u8,
    pub irr1: u8,
    pub irr2: u8,
    pub mode: Mode,
    pub stabilizing_from: Option<Mode>,
    irq_levels: [Option<bool>; 2],
    irq_clear_delay: [u8; 3],
    nmi_level: bool,
    nmi_pending: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            sys1: 3,
            sys2: 0xf0,
            osc: 0,
            feedback_cut: false,
            gate1: 3,
            gate2: 4,
            iegr: 0,
            ien1: 0,
            ien2: 0,
            irr1: 0,
            irr2: 0,
            mode: Mode::Active,
            stabilizing_from: None,
            irq_levels: [None; 2],
            irq_clear_delay: [0; 3],
            nmi_level: true,
            nmi_pending: false,
        }
    }
}
impl Control {
    pub fn reset(&mut self) {
        let nmi_level = self.nmi_level;
        *self = Self::default();
        self.nmi_level = nmi_level; // a reset does not drive an external input
    }
    pub fn nmi_level(&self) -> bool {
        self.nmi_level
    }
    pub fn nmi_pending(&self) -> bool {
        self.nmi_pending
    }
    pub fn acknowledge_nmi(&mut self) {
        self.nmi_pending = false;
    }
    pub fn nmi_input(&mut self, high: bool, enabled: bool) {
        if enabled && high != self.nmi_level && high == (self.iegr & 0x80 != 0) {
            self.nmi_pending = true;
        }
        self.nmi_level = high;
    }
    pub fn handles(a: u16) -> bool {
        matches!(a, 0xfff0..=0xfff7 | 0xfffa | 0xfffb)
    }
    pub fn read(&self, a: u16) -> u8 {
        match a {
            0xfff0 => self.sys1,
            0xfff1 => self.sys2,
            0xfff2 => self.iegr,
            0xfff3 => self.ien1,
            0xfff4 => self.ien2,
            0xfff5 => self.osc,
            0xfff6 => self.irr1,
            0xfff7 => self.irr2,
            0xfffa => self.gate1,
            _ => self.gate2,
        }
    }
    pub fn write(&mut self, a: u16, v: u8) -> Result<(), Error> {
        match a {
            0xfff0 => self.sys1 = v,
            0xfff1 => self.sys2 = v | 0xe0,
            0xfff2 => self.iegr = v & 0xa3,
            0xfff3 => self.ien1 = v & 0x87,
            0xfff4 => self.ien2 = v & 0x45,
            // OSCF is the latched E7_2 reset strap. The board selects the
            // main crystal; writing this status bit cannot select ROSC.
            0xfff5 => self.osc = (self.osc & 2) | (v & 0xe0),
            0xfff6 => {
                let protected = self
                    .irq_clear_delay
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (i, delay)| bits | (u8::from(*delay != 0) << i));
                self.irr1 &= (v | protected) & 7;
            }
            0xfff7 => self.irr2 &= v & 0x45,
            0xfffa => self.gate1 = v & 0x57,
            0xfffb => self.gate2 = v & 0x7e,
            _ => {
                return Err(Error::Unmapped {
                    address: a,
                    write: true,
                    width: 1,
                })
            }
        }
        Ok(())
    }
    pub fn pins(&mut self, levels: [Option<bool>; 2]) {
        for (i, level) in levels.iter().enumerate() {
            if self.irq_levels[i].is_none() && *level == Some(false) {
                self.irq_switch(i, true);
            }
            if let (Some(old), Some(new)) = (self.irq_levels[i], *level) {
                let rising = self.iegr & (1 << i) != 0;
                if old != new && new == rising {
                    self.irr1 |= 1 << i;
                }
            }
        }
        self.irq_levels = levels;
    }
    pub(crate) fn instruction_boundary(&mut self) {
        for delay in &mut self.irq_clear_delay {
            *delay = delay.saturating_sub(1);
        }
    }
    pub(crate) fn irq_switch(&mut self, index: usize, low: bool) {
        if low {
            self.irr1 |= 1 << index;
            // The selecting instruction's boundary, then one intervening instruction.
            self.irq_clear_delay[index] = 2;
        }
    }
    pub(crate) fn irq_routes_changed(&mut self, changed: [bool; 2]) {
        for (i, changed) in changed.into_iter().enumerate() {
            if changed {
                self.irq_switch(i, self.irq_levels[i] == Some(false));
                self.irq_levels[i] = None;
            }
        }
    }
    pub fn main_running(&self) -> bool {
        self.stabilizing_from.is_none() && matches!(self.mode, Mode::Active | Mode::Sleep)
    }
    pub fn sub_running(&self) -> bool {
        self.stabilizing_from.is_none() && matches!(self.mode, Mode::Subactive | Mode::Subsleep)
    }
    pub fn sleeping(&self) -> bool {
        matches!(
            self.mode,
            Mode::Sleep | Mode::Subsleep | Mode::Watch | Mode::Standby
        )
    }
    fn select_clock(&self, now: Time, c: &mut Clocks) -> Result<(), Error> {
        if self.stabilizing_from.is_none() && matches!(self.mode, Mode::Subactive | Mode::Subsleep)
        {
            c.select_cpu(now, true)
        } else {
            let divide = if self.sys2 & 4 != 0 {
                [8, 16, 32, 64][usize::from(self.sys1 & 3)]
            } else {
                1
            };
            c.select_system(now, Source::Oscillator, divide)?;
            c.select_cpu(now, false)
        }
    }
    /// Enter the intermediate halt mode. A direct transition subsequently
    /// wakes through the same clock and retention rules as an interrupt.
    pub fn sleep(&mut self, now: Time, c: &mut Clocks) -> Result<(), Error> {
        c.select_subclock(now, [8, 4, 2, 1][usize::from(self.sys2 & 3)])?;
        let standby = self.sys1 & 0x80 != 0;
        let watch = self.sys1 & 4 != 0;
        self.mode = if standby {
            if watch {
                Mode::Watch
            } else {
                Mode::Standby
            }
        } else if self.sub_running() {
            Mode::Subsleep
        } else {
            Mode::Sleep
        };
        if matches!(self.mode, Mode::Watch | Mode::Standby | Mode::Subactive) {
            self.feedback_cut = self.osc & 0x40 != 0;
        }
        if matches!(self.mode, Mode::Sleep | Mode::Subsleep) {
            self.select_clock(now, c)?;
        }
        Ok(())
    }
    pub fn wake(&mut self, now: Time, c: &mut Clocks) -> Result<Option<ClockWait>, Error> {
        let old = self.mode;
        self.mode = match old {
            Mode::Sleep => Mode::Active,
            Mode::Subsleep => Mode::Subactive,
            Mode::Watch if self.sys1 & 8 != 0 => Mode::Subactive,
            Mode::Watch | Mode::Standby => Mode::Active,
            m => m,
        };
        if self.mode == Mode::Subactive {
            self.feedback_cut = self.osc & 0x40 != 0;
        }
        if matches!(old, Mode::Watch | Mode::Standby) && self.mode == Mode::Active {
            self.stabilizing_from = Some(old);
            c.restart_oscillator(now)?;
            self.select_clock(now, c)?;
            let edges =
                [8192, 16384, 1024, 2048, 4096, 256, 512, 16][usize::from(self.sys1 >> 4 & 7)];
            Ok(Some(ClockWait::after(now, edges, Tap::oscillator(), c)?))
        } else {
            self.select_clock(now, c)?;
            Ok(None)
        }
    }
    pub fn synchronize_clock(&self, now: Time, c: &mut Clocks) -> Result<(), Error> {
        c.select_subclock(now, [8, 4, 2, 1][usize::from(self.sys2 & 3)])?;
        self.select_clock(now, c)
    }
}

impl Control {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        crate::state::require(
            self.sys2 & 0xe0 == 0xe0
                && self.osc & !0xe2 == 0
                && self.gate1 & !0x57 == 0
                && self.gate2 & !0x7e == 0
                && self.iegr & !0xa3 == 0
                && self.ien1 & !0x87 == 0
                && self.ien2 & !0x45 == 0
                && self.irr1 & !7 == 0
                && self.irr2 & !0x45 == 0
                && self.irq_clear_delay.iter().all(|d| *d <= 2),
            "invalid MCU control latches",
        )
    }
}
