pub mod adc;
pub mod aec;
pub mod clocks;
pub mod comparators;
pub mod control;
pub mod flash;
pub mod gpio;
pub mod iic;
pub mod rtc;
pub(crate) mod schedule;
pub mod sci;
pub mod ssu;
pub mod timer_b1;
pub mod timer_w;
pub mod watchdog;

use crate::{
    cpu::{Width, WriteOrigin},
    error::Error,
    signals::Output,
    time::Time,
};
use adc::Adc;
use aec::Aec;
use clocks::{Clocks, Frequencies, Tap};
use comparators::Comparators;
use control::{Control, Mode};
use flash::Flash;
use gpio::Gpio;
use iic::Iic;
use rtc::Rtc;
use sci::Sci;
use ssu::Ssu;
use timer_b1::TimerB1;
use timer_w::TimerW;
use watchdog::Watchdog;
pub const FLASH_SIZE: usize = 49_152;
pub const RAM_START: u16 = 0xf780;
pub const RAM_SIZE: usize = 2048;

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Mcu {
    pub(crate) flash: Flash,
    pub(crate) ram: [u8; RAM_SIZE],
    pub clocks: Clocks,
    pub control: Control,
    pub gpio: Gpio,
    pub rtc: Rtc,
    pub ssu: Ssu,
    pub sci: Sci,
    pub iic: Iic,
    pub timer_b1: TimerB1,
    pub timer_w: TimerW,
    pub watchdog: Watchdog,
    pub adc: Adc,
    pub aec: Aec,
    pub comparators: Comparators,
    reset_held: bool,
    startup: clocks::startup::Startup,
    /// Cleared enables still qualify surviving sources at this instruction's admission point.
    admission_enables: [u8; 10],
}
impl Mcu {
    pub fn new(firmware: &[u8], frequencies: Frequencies) -> Result<Self, Error> {
        let mut m = Self {
            flash: Flash::new(firmware)?,
            ram: [0; RAM_SIZE],
            clocks: Clocks::new(Time::ZERO, frequencies)?,
            control: Control::default(),
            gpio: Gpio::default(),
            rtc: Rtc::default(),
            ssu: Ssu::default(),
            sci: Sci::default(),
            iic: Iic::default(),
            timer_b1: TimerB1::default(),
            timer_w: TimerW::default(),
            watchdog: Watchdog::default(),
            adc: Adc::default(),
            aec: Aec::default(),
            comparators: Comparators::default(),
            reset_held: false,
            startup: Default::default(),
            admission_enables: [0; 10],
        };
        m.apply_gates(Time::ZERO, &mut ())?;
        Ok(m)
    }
    pub fn firmware(&self, now: Time) -> Box<[u8; FLASH_SIZE]> {
        self.flash.image(now)
    }
    pub fn ram(&self) -> &[u8; RAM_SIZE] {
        &self.ram
    }
    /// Synchronize clocked counters at an actual effect boundary. The return
    /// flag requests an MCU reset; attached device owners are not reconstructed.
    pub fn sync(&mut self, now: Time, out: &mut dyn Output) -> Result<bool, Error> {
        self.sync_peripherals(schedule::ALL, now, out)
    }
    pub fn collect_aec_requests(&mut self) {
        let requests = self.aec.take_requests();
        if requests & 1 != 0 {
            self.control.irr2 |= 1;
        }
        if requests & 2 != 0 {
            self.control.irr1 |= 4;
        }
    }
    fn aec_power(&mut self, now: Time) -> Result<(), Error> {
        let pad = self.control.mode != Mode::Standby
            && self.control.stabilizing_from != Some(Mode::Standby);
        let pwm = if self.aec.pwm_uses_watch() {
            pad && self.clocks.available(Tap::watch(1))
        } else {
            self.control.main_running()
        };
        self.aec.set_power(
            self.control.gate2 & 8 != 0,
            self.control.main_running(),
            pwm,
            pad,
            now,
            &self.clocks,
        )?;
        self.collect_aec_requests();
        Ok(())
    }
    pub fn apply_gates(&mut self, now: Time, out: &mut dyn Output) -> Result<(), Error> {
        let watch_mode = self.control.mode != Mode::Standby
            && self.control.stabilizing_from != Some(Mode::Standby);
        let main = self.control.main_running();
        let watch_on_chip = self.control.osc & 0x20 != 0;
        // Table 5.3 leaves X1 under SUBSTP even in standby. The W divider
        // and consumers halt separately; a stopped consumer cannot rephase X1.
        let crystal = self.control.osc & 0x80 == 0;
        let on_chip = self.reset_held
            || self.watchdog.rosc_for_module(self.control.gate2 & 4 != 0)
            || (self.control.mode != Mode::Standby && watch_on_chip);
        let power = self.startup.qualify(
            clocks::SourcePower {
                main,
                oscillator: main || self.control.stabilizing_from.is_some(),
                watch: if watch_on_chip { on_chip } else { crystal },
                crystal,
                on_chip,
                watch_on_chip,
            },
            now,
            self.control.osc & 2 != 0,
        )?;
        self.clocks.power_sources(now, power)?;
        // Physical rail loss freezes retained logic; it is not a register
        // module-stop write and must not reset shifters or counters.
        if !self.startup.supplied() {
            return Ok(());
        }
        self.flash.environment(
            self.control.mode,
            self.control.gate1 & 2 != 0,
            self.control.main_running()
                && self.control.osc & 2 == 0
                && self.clocks.available(Tap::oscillator()),
            now,
            out,
        )?;
        self.clocks.set_prescalers(
            now,
            !self.reset_held && self.control.main_running(),
            !self.reset_held
                && self.control.mode != Mode::Standby
                && self.control.stabilizing_from != Some(Mode::Standby),
        )?;
        self.comparators
            .set_gate(self.control.gate2 & 2 != 0, now)?;
        self.aec_power(now)?;
        let standby = self.control.mode == Mode::Standby;
        let main = !self.reset_held && self.control.main_running();
        let sub = !self.reset_held && self.control.sub_running();
        self.rtc.set_gate(
            !standby
                && self.control.gate1 & 1 != 0
                && (main || self.rtc.uses_watch())
                && (!self.rtc.uses_watch() || (watch_mode && self.clocks.available(Tap::watch(1)))),
            now,
            &self.clocks,
        );
        self.timer_b1.set_gate(
            !standby
                && self.control.gate1 & 4 != 0
                && (main || self.timer_b1.uses_watch())
                && (!self.timer_b1.uses_watch()
                    || (watch_mode && self.clocks.available(Tap::watch(1)))),
            now,
            &self.clocks,
        );
        self.timer_w.set_gate(
            self.control.gate2 & 0x40 != 0
                && self.control.stabilizing_from.is_none()
                && (main || (sub && (self.timer_w.uses_watch() || self.timer_w.uses_external()))),
            now,
            &self.clocks,
        )?;
        let watchdog_source = match self.watchdog.source() {
            clocks::Source::System => main,
            clocks::Source::Watch => {
                !standby
                    && self.control.stabilizing_from != Some(Mode::Standby)
                    && self.clocks.available(Tap::watch(1))
            }
            _ => true,
        };
        self.watchdog.set_power(
            self.control.gate2 & 4 != 0,
            watchdog_source,
            self.reset_held,
            now,
            &self.clocks,
        );
        self.ssu.set_gate(
            (main || (sub && self.ssu.uses_subclock())) && self.control.gate2 & 0x10 != 0,
            now,
            &self.clocks,
        )?;
        self.iic
            .set_gate(main && self.control.gate2 & 0x20 != 0, now, &self.clocks)?;
        self.sci.set_power(
            self.control.gate1 & 0x40 != 0,
            main,
            sub,
            matches!(self.control.mode, Mode::Watch | Mode::Standby)
                || self.control.stabilizing_from.is_some(),
            now,
            &self.clocks,
        )?;
        self.adc.set_gate(
            self.control.gate1 & 0x10 != 0 && (main || (sub && self.adc.uses_watch())),
            now,
            &self.clocks,
        )?;
        Ok(())
    }
    pub(crate) fn set_supply(
        &mut self,
        millivolts: u16,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        let was = self.startup.supplied();
        self.startup.set_rail(millivolts);
        let supplied = self.startup.supplied();
        if was && !supplied {
            self.flash.power_off(now, out);
            self.sci.supply_lost();
        }
        self.comparators.set_supply(supplied, now)?;
        self.apply_gates(now, out)
    }
    pub(crate) fn lose_volatile(&mut self, now: Time, out: &mut dyn Output) -> Result<(), Error> {
        self.rtc = Rtc::default();
        self.ram.fill(0);
        self.reset(now, false, out)
    }
    pub fn reset(&mut self, now: Time, watchdog: bool, out: &mut dyn Output) -> Result<(), Error> {
        // RAM, flash, watch-source phase, RTC, and external chips survive an MCU
        // reset. Undefined MCU RAM is initialized only by cold construction.
        // Settle retained counters before changing their clock reference points.
        self.sync(now, out)?;
        self.flash.reset(now, out);
        if !self.control.main_running() {
            self.clocks.restart_oscillator(now)?;
        }
        if !self.reset_held && !self.watchdog.rosc_required() && self.control.osc & 0x22 == 0 {
            self.clocks.restart_on_chip(now)?;
        }
        self.clocks.reset_prescalers(now)?;
        self.control.reset();
        self.admission_enables = [0; 10];
        self.control.synchronize_clock(now, &mut self.clocks)?;
        self.gpio.reset();
        self.ssu = Ssu::default();
        self.sci = Sci::default();
        self.iic = Iic::default();
        self.timer_b1 = TimerB1::default();
        self.timer_w = TimerW::default();
        self.adc.reset();
        self.rtc.reset(now, &self.clocks);
        self.aec = Aec::default();
        self.comparators.reset(now);
        self.watchdog.reset(watchdog, now, &self.clocks);
        self.apply_gates(now, out)
    }
    pub fn hold_reset(&mut self, held: bool, now: Time, out: &mut dyn Output) -> Result<(), Error> {
        self.reset_held = held;
        self.apply_gates(now, out)
    }
    fn pin_clock(&self) -> Option<(Tap, bool)> {
        let tap = match self.gpio.clock_selection() {
            2 | 3 => self.rtc.output_tap(),
            selection @ 4..=6 => Tap {
                source: clocks::Source::Oscillator,
                divide: 1 << (selection - 4),
            },
            _ => return None,
        };
        let running = match tap.source {
            clocks::Source::Oscillator => true,
            clocks::Source::System => self.control.gate1 & 1 != 0 && self.control.main_running(),
            _ => {
                self.control.gate1 & 1 != 0
                    && self.control.mode != Mode::Standby
                    && self.control.stabilizing_from != Some(Mode::Standby)
            }
        } && self.clocks.available(tap);
        Some((tap, running))
    }
    pub fn update_clock_output(&mut self, now: Time) {
        if self.gpio.clock_selection() < 2 {
            return;
        }
        let floating = self.control.mode == Mode::Standby
            || self.control.stabilizing_from == Some(Mode::Standby)
            || self.gpio.clock_selection() == 7;
        let level = self
            .pin_clock()
            .and_then(|(tap, running)| (running && !floating).then(|| self.clocks.high(now, tap)));
        self.gpio.set_clock_output(level, floating);
    }
    pub fn clock_output_deadline(&self, now: Time) -> Result<Option<Time>, Error> {
        if !self.startup.supplied()
            || self.control.mode == Mode::Standby
            || self.control.stabilizing_from == Some(Mode::Standby)
        {
            return Ok(None);
        }
        match self.pin_clock() {
            Some((tap, true)) => self.clocks.next_transition(now, tap),
            _ => Ok(None),
        }
    }
    pub fn interrupt(&self) -> Option<u8> {
        if self.control.nmi_pending() {
            return Some(7);
        }
        let mut best = None;
        let mut push = |v: u8| {
            best = Some(best.map_or(v, |old: u8| old.min(v)));
        };
        let retained = self.admission_enables;
        let ext = self.control.irr1 & (self.control.ien1 | retained[0]);
        if ext & 1 != 0 {
            push(16);
        }
        if ext & 2 != 0 {
            push(17);
        }
        if ext & 4 != 0 {
            push(18);
        }
        if (self.control.ien1 | retained[0]) & 0x80 != 0 {
            if let Some(v) = self.rtc.interrupt_with_enable(retained[2]) {
                push(v);
            }
        }
        if self.watchdog.interrupt_with_enable(retained[3]) {
            push(31);
        }
        let request = self.control.irr2 & (self.control.ien2 | retained[1]);
        if request & 1 != 0 {
            push(32);
        }
        if request & 4 != 0 {
            push(33);
        }
        if self.ssu.interrupt_with_enable(retained[4])
            || self.iic.interrupt_with_enable(retained[5])
        {
            push(34);
        }
        if self.timer_w.interrupt_with_enable(retained[6]) {
            push(35);
        }
        if let Some(vector) = self
            .comparators
            .interrupt_with_enable([retained[7], retained[8]])
        {
            push(vector);
        }
        if self.sci.interrupt_with_enable(retained[9]) {
            push(37);
        }
        if request & 0x40 != 0 {
            push(38);
        }
        best
    }
    pub(crate) fn instruction_boundary(&mut self) {
        self.admission_enables = [0; 10];
        self.control.instruction_boundary();
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
    pub(crate) fn read_starts_transfer(a: u16) -> bool {
        // Reading SSURDR or ICDRR can start reception or release a held clock.
        matches!(a, 0xf0e9 | 0xf07f)
    }
    pub fn read8(&mut self, a: u16, now: Time, out: &mut dyn Output) -> Result<u8, Error> {
        if a < 0xc000 {
            return self.flash.read8(a, now, out);
        }
        if (RAM_START..=0xff7f).contains(&a) {
            return Ok(self.ram[usize::from(a - RAM_START)]);
        }
        if a >= 0xc000 && Self::native_word(a & !1) {
            let word = self.word_value(a & !1)?;
            return Ok((word >> if a & 1 == 0 { 8 } else { 0 }) as u8);
        }
        if a == 0xffde {
            let channel = self.adc.channel();
            let adc_mask = if (4..=9).contains(&channel) {
                1 << (channel - 4)
            } else {
                0
            };
            return Ok(self.gpio.read(a) & !adc_mask);
        }
        if Gpio::handles(a) {
            return Ok(self.gpio.read(a));
        }
        if Control::handles(a) {
            return Ok(self.control.read(a));
        }
        if Sci::handles(a) {
            return Ok(self.sci.read(a));
        }
        if Aec::handles(a) {
            return Ok(self.aec.read(a));
        }
        match a {
            0xf067..=0xf06d | 0xf06f => Ok(self.rtc.read(a)),
            0xf0dc..=0xf0de => Ok(self.comparators.read(a)),
            0xf0d0 => Ok(self.timer_b1.read(a)),
            0xf0d1 => Ok(self.timer_b1.read(a)),
            0xf0e0..=0xf0e4 | 0xf0e9 | 0xf0eb => self.ssu.read(a, now, &self.clocks),
            0xf0f0..=0xf0f5 => Ok(self.timer_w.read(a)),
            0xffb0..=0xffb3 => Ok(self.watchdog.read(a)),
            0xffbe | 0xffbf => Ok(self.adc.peek(a)),
            0xf078..=0xf07f => self.iic.read(a, now, &self.clocks),
            0xf020..=0xf023 | 0xf02b => Ok(self.flash.register(a)),
            _ => Ok(0),
        }
    }
    pub fn read16(&mut self, a: u16, now: Time, out: &mut dyn Output) -> Result<u16, Error> {
        let a = a & !1;
        if a < 0xc000 {
            return self.flash.read16(a, now, out);
        }
        self.word_value(a)
    }
    fn word_value(&self, a: u16) -> Result<u16, Error> {
        if (RAM_START..=0xff7e).contains(&a) {
            let i = usize::from(a - RAM_START);
            return Ok(u16::from_be_bytes([self.ram[i], self.ram[i + 1]]));
        }
        match a {
            0xf0f6 | 0xf0f8 | 0xf0fa | 0xf0fc | 0xf0fe => self.timer_w.read_word(a, &self.clocks),
            0xffbc => Ok(self.adc.result()),
            0xff8c | 0xff8e => self.aec.read_word(a),
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
            let before = self.peek8(a)?;
            self.write_byte(a, v, origin, now, out)?;
            self.admission_enables[index] |= before & !self.peek8(a)? & mask;
            Ok(())
        } else {
            self.write_byte(a, v, origin, now, out)
        }
    }
    fn write_byte(
        &mut self,
        a: u16,
        v: u8,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        if (RAM_START..=0xff7f).contains(&a) {
            self.ram[usize::from(a - RAM_START)] = v;
            return Ok(());
        }
        if a < 0xc000 {
            return self.flash.write8(a, v, now, out);
        }
        // These latches require a native word write strobe. Byte stores do
        // not synthesize a read-modify-write of an unspecified other lane.
        if a >= 0xc000 && Self::native_word(a & !1) {
            return Ok(());
        }
        if Gpio::handles(a) {
            let before = self.gpio.irq_routes();
            self.gpio.write(a, v)?;
            let after = self.gpio.irq_routes();
            self.control
                .irq_routes_changed([before[0] != after[0], before[1] != after[1]]);
            return Ok(());
        }
        if Control::handles(a) {
            self.control.write(a, v)?;
            return self.apply_gates(now, out);
        }
        if Sci::handles(a) {
            return self.sci.write(a, v, now, &self.clocks);
        }
        if Aec::handles(a) {
            let was_pwm = self.aec.pwm_enabled();
            let old_gate = self.aec.gate();
            self.aec.write(a, v, now, &self.clocks)?;
            if was_pwm != self.aec.pwm_enabled() {
                self.control.irq_switch(2, !old_gate || !self.aec.gate());
            }
            return self.aec_power(now);
        }
        match a {
            0xf067..=0xf06d | 0xf06f => {
                self.rtc.write(a, v, now, &self.clocks)?;
                if a == 0xf06f {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            0xf0dc..=0xf0de => self.comparators.write(a, v, now),
            0xf0d0..=0xf0d1 => {
                if self.timer_b1.write(a, v, now, &self.clocks)? {
                    self.control.irr2 |= 4;
                }
                if a == 0xf0d0 {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            0xf0e0..=0xf0e4 | 0xf0e9 | 0xf0eb => {
                self.ssu.write(a, v, origin.is_mov(), now, &self.clocks)?;
                if a == 0xf0e2 {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            0xf0f0..=0xf0f5 => {
                self.timer_w.write(a, v, now, &self.clocks)?;
                if a == 0xf0f1 {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            0xffb0..=0xffb3 => {
                self.watchdog.write(a, v, origin, now, &self.clocks)?;
                self.apply_gates(now, out)
            }
            0xffbe | 0xffbf => {
                self.adc.write(a, v, now, &self.clocks)?;
                if a == 0xffbe {
                    self.apply_gates(now, out)
                } else {
                    Ok(())
                }
            }
            0xf078..=0xf07f => self.iic.write(a, v, now, &self.clocks),
            0xf020..=0xf023 | 0xf02b => {
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
        let a = a & !1;
        if a < 0xc000 {
            let bytes = v.to_be_bytes();
            self.flash.write8(a, bytes[0], now, out)?;
            return self.flash.write8(a + 1, bytes[1], now, out);
        }
        if (RAM_START..=0xff7e).contains(&a) {
            let i = usize::from(a - RAM_START);
            self.ram[i..i + 2].copy_from_slice(&v.to_be_bytes());
            return Ok(());
        }
        match a {
            0xf0f6 | 0xf0f8 | 0xf0fa | 0xf0fc | 0xf0fe => {
                self.timer_w.write_word(a, v, now, &self.clocks)
            }
            0xffbc => Ok(()),
            0xff8c | 0xff8e => {
                self.aec.write_word(a, v, now, &self.clocks)?;
                self.collect_aec_requests();
                Ok(())
            }
            _ => Ok(()),
        }
    }
    pub fn peek8(&self, a: u16) -> Result<u8, Error> {
        if a < 0xc000 {
            return Ok(self.flash.settled_byte(a));
        }
        if (RAM_START..=0xff7f).contains(&a) {
            return Ok(self.ram[usize::from(a - RAM_START)]);
        }
        if a >= 0xc000 && Self::native_word(a & !1) {
            let word = self.word_value(a & !1)?;
            return Ok((word >> if a & 1 == 0 { 8 } else { 0 }) as u8);
        }
        if a == 0xffde {
            let channel = self.adc.channel();
            let adc_mask = if (4..=9).contains(&channel) {
                1 << (channel - 4)
            } else {
                0
            };
            return Ok(self.gpio.read(a) & !adc_mask);
        }
        if Gpio::handles(a) {
            return Ok(self.gpio.read(a));
        }
        if Control::handles(a) {
            return Ok(self.control.read(a));
        }
        if Sci::handles(a) {
            return Ok(self.sci.peek(a));
        }
        if Aec::handles(a) || (0xff8c..=0xff8f).contains(&a) {
            return Ok(self.aec.peek(a));
        }
        match a {
            0xf067..=0xf06d | 0xf06f => Ok(self.rtc.read(a)),
            0xf0dc..=0xf0de => Ok(self.comparators.peek(a)),
            0xf0d0 => Ok(self.timer_b1.read(a)),
            0xf0d1 => Ok(self.timer_b1.read(a)),
            0xf0e0..=0xf0e4 | 0xf0e9 | 0xf0eb => Ok(self.ssu.peek(a)),
            0xf0f0..=0xf0ff => Ok(self.timer_w.peek(a)),
            0xffb0..=0xffb3 => Ok(self.watchdog.peek(a)),
            0xffbe | 0xffbf => Ok(self.adc.peek(a)),
            0xf078..=0xf07f => Ok(self.iic.peek(a)),
            0xffbc => Ok((self.adc.result() >> 8) as u8),
            0xffbd => Ok(self.adc.result() as u8),
            0xf020..=0xf023 | 0xf02b => Ok(self.flash.register(a)),
            _ => Ok(0),
        }
    }
    pub fn delay(&self, now: Time, states: u64) -> Result<Time, Error> {
        self.clocks.after(now, states, Tap::system(1))
    }
}

impl Mcu {
    pub(crate) fn validate(&mut self, now: Time, faulted: bool) -> Result<(), Error> {
        self.clocks.validate(now)?;
        self.flash.validate(now)?;
        self.control.validate()?;
        self.gpio.validate()?;
        self.rtc.validate()?;
        self.ssu.validate(faulted)?;
        self.sci.validate(now)?;
        self.iic.validate(now)?;
        self.timer_b1.validate()?;
        self.timer_w.validate(now)?;
        self.watchdog.validate(now, &self.clocks)?;
        self.adc.validate()?;
        self.aec.validate(now)?;
        self.comparators.validate(now)?;
        self.startup.validate(now)?;
        crate::state::future(self.deadline()?, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtc_free_counter_enable_survives_its_clearing_instruction() {
        let mut mcu = Mcu::new(&[0; FLASH_SIZE], Frequencies::default()).unwrap();
        for (address, value) in [(0xfff3, 0x80), (0xf06f, 0), (0xf06d, 0x80), (0xf06c, 0x80)] {
            mcu.write8(address, value, WriteOrigin::Other, Time::ZERO, &mut ())
                .unwrap();
        }
        let now = Time::from_micros(1000);
        mcu.sync(now, &mut ()).unwrap();
        assert_eq!(mcu.peek8(0xf067).unwrap() & 0x80, 0x80);
        assert_eq!(mcu.interrupt(), Some(30));
        mcu.write8(0xf06d, 0, WriteOrigin::Other, now, &mut ())
            .unwrap();
        assert_eq!(mcu.peek8(0xf06d).unwrap(), 0);
        assert_eq!(mcu.interrupt(), Some(30));
        mcu.instruction_boundary();
        assert_eq!(mcu.interrupt(), None);
        assert_eq!(mcu.peek8(0xf067).unwrap() & 0x80, 0x80);
    }
}
