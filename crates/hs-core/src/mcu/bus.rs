//! Address routing for physical MCU transactions. Peripheral owners retain
//! register semantics; this disposable description carries routing across timing
//! and commitment without adding captured hardware state.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Target {
    Flash,
    Ram,
    Gpio,
    Control,
    Sci,
    Aec,
    Rtc,
    Comparators,
    TimerB1,
    Ssu,
    TimerW,
    Watchdog,
    Adc,
    Iic,
    FlashControl,
    Unmapped,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Access {
    pub address: u16,
    pub owners: u16,
    pub width: Width,
    target: Target,
    states: u8,
    flags: u8,
}
/// A completed physical owner transaction awaiting connection/appointment
/// settlement. This transient reply is never part of captured CPU state.
pub(crate) struct Committed {
    pub access: Access,
    pub value: u16,
}

impl Access {
    pub fn memory(self) -> bool {
        matches!(self.target, Target::Flash | Target::Ram)
    }
    pub fn native_word(self) -> bool {
        self.flags & 1 != 0
    }
    pub fn configures_board(self) -> bool {
        self.flags & 2 != 0
    }
    pub fn read_changes_state(self) -> bool {
        self.flags & 4 != 0
    }
    pub(super) fn ssu_data(self, write: bool) -> bool {
        self.width == Width::Byte
            && self.target == Target::Ssu
            && Ssu::retained_data(self.address, write)
    }
    pub(super) fn retained_write(self) -> bool {
        self.width == Width::Byte && self.target == Target::Gpio
    }
    pub(crate) fn observes_serial(self) -> bool {
        self.flags & 8 != 0
    }
    pub fn states(self) -> u64 {
        u64::from(self.states)
    }
}

pub(super) const MEMORY_STATES: u8 = 2;
#[inline]
pub(super) fn memory_target(a: u16) -> Option<Target> {
    if a < 0xc000 {
        Some(Target::Flash)
    } else if (RAM_START..=0xff7f).contains(&a) {
        Some(Target::Ram)
    } else {
        None
    }
}

impl Mcu {
    pub(crate) fn classify(address: u16, width: Width, write: bool) -> Access {
        let a = if width == Width::Word {
            address & !1
        } else {
            address
        };
        let memory = memory_target(a);
        if let Some(target) = memory {
            return Access {
                address: a,
                width,
                target,
                owners: 0,
                states: MEMORY_STATES,
                flags: 1,
            };
        }
        let target = if Gpio::handles(a) {
            Target::Gpio
        } else if Control::handles(a) {
            Target::Control
        } else if Sci::handles(a) {
            Target::Sci
        } else if Aec::handles(a) || matches!(a & !1, 0xff8c | 0xff8e) {
            Target::Aec
        } else {
            match a {
                0xf067..=0xf06d | 0xf06f => Target::Rtc,
                0xf0dc..=0xf0de => Target::Comparators,
                0xf0d0..=0xf0d1 => Target::TimerB1,
                0xf0e0..=0xf0e4 | 0xf0e9 | 0xf0eb => Target::Ssu,
                0xf0f0..=0xf0ff => Target::TimerW,
                0xffb0..=0xffb3 => Target::Watchdog,
                0xffbc..=0xffbf => Target::Adc,
                0xf078..=0xf07f => Target::Iic,
                0xf020..=0xf023 | 0xf02b => Target::FlashControl,
                _ => Target::Unmapped,
            }
        };
        let owners = match target {
            Target::Rtc => schedule::RTC,
            Target::TimerB1 => schedule::TIMER_B1,
            Target::TimerW => schedule::TIMER_W,
            Target::Watchdog => schedule::WATCHDOG,
            Target::Ssu => schedule::SSU,
            Target::Sci => schedule::SCI,
            Target::Iic => schedule::IIC,
            Target::Adc => schedule::ADC,
            Target::Comparators => schedule::COMPARATORS,
            Target::Aec => schedule::AEC,
            // Unassigned cells in an owner's register bank still settle it.
            Target::Unmapped if a == 0xf06e => schedule::RTC,
            Target::Unmapped if (0xf0e0..=0xf0eb).contains(&a) => schedule::SSU,
            _ => 0,
        };
        let shared_clocks = write
            && (Control::clock_register(a)
                || matches!(
                    a,
                    0xf06f | 0xf0d0 | 0xf0e2 | 0xf0f1 | 0xffb0..=0xffb3 | 0xffbe | 0xf022
                )
                || Aec::handles(a));
        let configuration = matches!(
            target,
            Target::Gpio | Target::Control | Target::Sci | Target::Comparators
        ) || Aec::handles(a)
            || matches!(a, 0xffbe | 0xffbf);
        Access {
            address: a,
            width,
            target,
            owners: if shared_clocks { schedule::ALL } else { owners },
            states: Self::access_states(a, width) as u8,
            flags: u8::from(Self::native_word(a & !1))
                | (u8::from(configuration) << 1)
                | (u8::from(Self::read_changes_state(a)) << 2)
                | (u8::from(
                    write
                        || match target {
                            Target::Gpio => Gpio::reads_serial(a),
                            Target::Ssu => Ssu::reads_shift(a),
                            Target::Iic => Self::read_changes_state(a),
                            _ => false,
                        },
                ) << 3),
        }
    }
    pub fn is_memory(a: u16) -> bool {
        a < 0xc000 || (RAM_START..=0xff7f).contains(&a)
    }
    pub fn native_word(a: u16) -> bool {
        Self::is_memory(a)
            || matches!(
                a,
                0xf0f6 | 0xf0f8 | 0xf0fa | 0xf0fc | 0xf0fe | 0xff8c | 0xff8e | 0xffbc
            )
    }
    /// Duration of one *physical* access, in reference-clock states.
    /// REJ09B0152-0300 §20.1 (pp.372–375): only SSU and the SCI core
    /// registers below take three states. SPCR and IrCR take TWO states.
    /// A logical word on an 8-bit bus is issued as two physical byte accesses
    /// by Machine; this function must not collapse it to one two-state access.
    pub fn access_states(a: u16, _width: Width) -> u64 {
        if matches!(a, 0xf0e0..=0xf0e4 | 0xf0e9 | 0xf0eb | 0xff98..=0xff9d | 0xffa6) {
            3
        } else {
            2
        }
    }
    pub(crate) fn read_changes_state(a: u16) -> bool {
        // Data reads can start reception, release a held clock or clear an
        // interrupt. CMPCR also resolves a coincident comparator event.
        matches!(a, 0xf0e9 | 0xf07f | 0xff9d | 0xf0de)
    }
    pub fn read8(&mut self, a: u16, now: Time, out: &mut dyn Output) -> Result<u8, Error> {
        self.read_access(Self::classify(a, Width::Byte, false), now, out)
            .map(|v| v as u8)
    }
    pub fn read16(&mut self, a: u16, now: Time, out: &mut dyn Output) -> Result<u16, Error> {
        self.read_access(Self::classify(a, Width::Word, false), now, out)
    }
    pub(crate) fn read_access(
        &mut self,
        access: Access,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<u16, Error> {
        self.read_access_at(access, || Ok(now), out)
    }
    // Retained register values do not consume time. Let the adapter project its
    // local edge ordinal only when a timed owner actually requests a timestamp.
    pub(super) fn read_access_at(
        &mut self,
        access: Access,
        now: impl FnOnce() -> Result<Time, Error>,
        out: &mut dyn Output,
    ) -> Result<u16, Error> {
        let a = access.address;
        if access.width == Width::Word {
            return match access.target {
                Target::Flash => self.flash.read16(a, now()?, out),
                _ => self.word_value(access),
            };
        }
        if access.native_word() && !access.memory() {
            let word = self.word_value(access)?;
            return Ok(u16::from((word >> if a & 1 == 0 { 8 } else { 0 }) as u8));
        }
        let value = match access.target {
            Target::Flash => self.flash.read8(a, now()?, out)?,
            Target::Ram => self.ram[usize::from(a - RAM_START)],
            Target::Gpio => self.gpio_value(a),
            Target::Control => self.control.read(a),
            Target::Sci => self.sci.read(a),
            Target::Aec => self.aec.read(a),
            Target::Rtc => self.rtc.read(a),
            Target::Comparators => self.comparators.read(a),
            Target::TimerB1 => self.timer_b1.read(a),
            Target::Ssu => self.ssu.read_at(a, now, &self.clocks)?,
            Target::TimerW => self.timer_w.read(a),
            Target::Watchdog => self.watchdog.read(a),
            Target::Adc => self.adc.peek(a),
            Target::Iic => self.iic.read_at(a, now, &self.clocks)?,
            Target::FlashControl => self.flash.register(a),
            Target::Unmapped => 0,
        };
        Ok(u16::from(value))
    }
    fn gpio_value(&self, a: u16) -> u8 {
        let mask = if a == 0xffde && (4..=9).contains(&self.adc.channel()) {
            1 << (self.adc.channel() - 4)
        } else {
            0
        };
        self.gpio.read(a) & !mask
    }
    fn word_value(&self, access: Access) -> Result<u16, Error> {
        if !access.native_word() {
            return Ok(0);
        }
        let a = access.address & !1;
        match access.target {
            Target::Ram => {
                let i = usize::from(a - RAM_START);
                Ok(u16::from_be_bytes([self.ram[i], self.ram[i + 1]]))
            }
            Target::TimerW => self.timer_w.read_word(a, &self.clocks),
            Target::Adc => Ok(self.adc.result()),
            Target::Aec => self.aec.read_word(a),
            _ => Ok(0),
        }
    }
    pub fn write8(
        &mut self,
        a: u16,
        v: u8,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        self.write_access(
            Self::classify(a, Width::Byte, true),
            u16::from(v),
            origin,
            now,
            out,
        )
        .map(|_| ())
    }
    // Retained GPIO writes need no clock projection. This one owner call also
    // records IRQ function-selection changes before the board resolves levels.
    pub(super) fn write_gpio(&mut self, address: u16, value: u8) -> Result<bool, Error> {
        let before = self.gpio.irq_routes();
        let changed = self.gpio.write_changed(address, value)?;
        let after = self.gpio.irq_routes();
        self.control
            .irq_routes_changed([before[0] != after[0], before[1] != after[1]]);
        Ok(changed)
    }
    pub(crate) fn write_access(
        &mut self,
        access: Access,
        value: u16,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<bool, Error> {
        if access.width == Width::Word {
            self.write_word(access, value, now, out)?;
            return Ok(true);
        }
        let a = access.address;
        let v = value as u8;
        if access.target == Target::Gpio {
            return self.write_gpio(a, v);
        }
        let field = match a {
            0xfff3 => Some((0, 0x87)),
            0xfff4 => Some((1, 0x45)),
            0xf06d => Some((2, 0xff)),
            0xffb2 => Some((3, 8)),
            0xf0e3 => Some((4, 15)),
            0xf07b => Some((5, 0xf8)),
            0xf0f2 => Some((6, 0x8f)),
            0xf0dc => Some((7, 0x40)),
            0xf0dd => Some((8, 0x40)),
            0xff9a => Some((9, 0xc4)),
            _ => None,
        };
        if let Some((index, mask)) = field {
            let before = self.peek_access(access)?;
            self.write_byte(access, v, origin, now, out)?;
            self.admission_enables[index] |= before & !self.peek_access(access)? & mask;
            Ok(true)
        } else {
            self.write_byte(access, v, origin, now, out).map(|_| true)
        }
    }
    fn write_byte(
        &mut self,
        access: Access,
        v: u8,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        let a = access.address;
        // Native word latches ignore byte stores rather than synthesize an
        // unspecified read-modify-write of the opposite lane.
        if access.native_word() && !access.memory() {
            return Ok(());
        }
        match access.target {
            Target::Ram => {
                self.ram[usize::from(a - RAM_START)] = v;
                Ok(())
            }
            Target::Flash => self.flash.write8(a, v, now, out),
            Target::Control => {
                self.control.write(a, v)?;
                if Control::clock_register(a) {
                    self.apply_gates(now, out)?;
                }
                Ok(())
            }
            Target::Sci => self.sci.write(a, v, now, &self.clocks),
            Target::Aec => {
                let was_pwm = self.aec.pwm_enabled();
                let old_gate = self.aec.gate();
                self.aec.write(a, v, now, &self.clocks)?;
                if was_pwm != self.aec.pwm_enabled() {
                    self.control.irq_switch(2, !old_gate || !self.aec.gate());
                }
                self.aec_power(now)
            }
            Target::Rtc => {
                self.rtc.write(a, v, now, &self.clocks)?;
                if a == 0xf06f {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            Target::Comparators => self.comparators.write(a, v, now),
            Target::TimerB1 => {
                if self.timer_b1.write(a, v, now, &self.clocks)? {
                    self.control.irr2 |= 4;
                }
                if a == 0xf0d0 {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            Target::Ssu => {
                self.ssu.write(a, v, origin.is_mov(), now, &self.clocks)?;
                if a == 0xf0e2 {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            Target::TimerW => {
                self.timer_w.write(a, v, now, &self.clocks)?;
                if a == 0xf0f1 {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            Target::Watchdog => {
                self.watchdog.write(a, v, origin, now, &self.clocks)?;
                self.apply_gates(now, out)
            }
            Target::Adc => {
                self.adc.write(a, v, now, &self.clocks)?;
                if a == 0xffbe {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            Target::Iic => self.iic.write(a, v, now, &self.clocks),
            Target::FlashControl => {
                self.flash.write_register(a, v, now, out)?;
                if a == 0xf022 {
                    self.apply_gates(now, out)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    pub fn write16(
        &mut self,
        a: u16,
        v: u16,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        self.write_word(Self::classify(a, Width::Word, true), v, now, out)
    }
    fn write_word(
        &mut self,
        access: Access,
        v: u16,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        if !access.native_word() {
            return Ok(());
        }
        let a = access.address;
        match access.target {
            Target::Flash => {
                let bytes = v.to_be_bytes();
                self.flash.write8(a, bytes[0], now, out)?;
                self.flash.write8(a + 1, bytes[1], now, out)
            }
            Target::Ram => {
                let i = usize::from(a - RAM_START);
                self.ram[i..i + 2].copy_from_slice(&v.to_be_bytes());
                Ok(())
            }
            Target::TimerW => self.timer_w.write_word(a, v, now, &self.clocks),
            Target::Aec => {
                self.aec.write_word(a, v, now, &self.clocks)?;
                self.collect_aec_requests();
                Ok(())
            }
            _ => Ok(()),
        }
    }
    pub fn peek8(&self, a: u16) -> Result<u8, Error> {
        self.peek_access(Self::classify(a, Width::Byte, false))
    }
    fn peek_access(&self, access: Access) -> Result<u8, Error> {
        let a = access.address;
        if access.native_word() && !access.memory() {
            let word = self.word_value(access)?;
            return Ok((word >> if a & 1 == 0 { 8 } else { 0 }) as u8);
        }
        Ok(match access.target {
            Target::Flash => self.flash.settled_byte(a),
            Target::Ram => self.ram[usize::from(a - RAM_START)],
            Target::Gpio => self.gpio_value(a),
            Target::Control => self.control.read(a),
            Target::Sci => self.sci.peek(a),
            Target::Aec => self.aec.peek(a),
            Target::Rtc => self.rtc.read(a),
            Target::Comparators => self.comparators.peek(a),
            Target::TimerB1 => self.timer_b1.read(a),
            Target::Ssu => self.ssu.peek(a),
            Target::TimerW => self.timer_w.peek(a),
            Target::Watchdog => self.watchdog.peek(a),
            Target::Adc => self.adc.peek(a),
            Target::Iic => self.iic.peek(a),
            Target::FlashControl => self.flash.register(a),
            Target::Unmapped => 0,
        })
    }
    pub fn delay(&self, now: Time, states: u64) -> Result<Time, Error> {
        self.clocks.after(now, states, Tap::system(1))
    }
}
