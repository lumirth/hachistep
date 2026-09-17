//! MCU operating modes, clock/module controls, and external interrupt latches.
//! Invalid mode combinations stop explicitly; there is no product-level wake
//! shortcut based on a button name or retail firmware address.
use super::clocks::{ClockWait, Clocks, Source, Tap};
use crate::{error::Error, time::Time};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Active,
    Subactive,
    Sleep,
    Subsleep,
    Watch,
    Standby,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub sys1: u8,
    pub sys2: u8,
    pub osc: u8,
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
    nmi_level: bool,
    nmi_pending: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            sys1: 3,
            sys2: 0xf0,
            osc: 0,
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
            0xfff5 => {
                if v & 0xc0 != 0 {
                    return Err(Error::Unsupported {
                        component: "clocks",
                        detail: "watch oscillator stop/external source switch is not implemented",
                        address: a,
                    });
                }
                self.osc = v & 0xe2;
            }
            0xfff6 => self.irr1 &= v & 7,
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
            if let (Some(old), Some(new)) = (self.irq_levels[i], *level) {
                let rising = self.iegr & (1 << i) != 0;
                if old != new && new == rising {
                    self.irr1 |= 1 << i;
                }
            }
        }
        self.irq_levels = levels;
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
            let divide = [8, 4, 2, 1][usize::from(self.sys2 & 3)];
            c.select_system(now, Source::Watch, divide)
        } else {
            let divide = if self.sys2 & 4 != 0 {
                [8, 16, 32, 64][usize::from(self.sys1 & 3)]
            } else {
                1
            };
            c.select_system(now, Source::Oscillator, divide)
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
