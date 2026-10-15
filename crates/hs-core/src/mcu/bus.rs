//! Address routing for physical MCU transactions. Peripheral owners retain
//! register semantics; this disposable description carries routing across timing
//! and commitment without adding captured hardware state.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Target {
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

impl Access {
    /// An SSU register, which the serial engine brings up to date.
    pub(crate) fn serial_engine(self) -> bool {
        self.target == Target::Ssu
    }
    pub fn memory(self) -> bool {
        matches!(self.target, Target::Flash | Target::Ram)
    }
    pub const fn native_word(self) -> bool {
        self.flags & 1 != 0
    }
    pub fn configures_board(self) -> bool {
        self.flags & 2 != 0
    }
    pub const fn read_changes_state(self) -> bool {
        self.flags & 4 != 0
    }
    pub(crate) fn observes_serial(self) -> bool {
        self.flags & 8 != 0
    }
    /// A read whose value can change only at its owner's appointments, and
    /// whose side effects a repeated read with the same value cannot change.
    pub(crate) fn settled(self) -> bool {
        self.flags & 16 != 0
    }
    pub fn states(self) -> u64 {
        u64::from(self.states)
    }
}

/// How a write can recur in an iteration that repeats with nothing else
/// changing (see `cpu::repetition`). The owner declares it per register.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Recurrence {
    /// The write's effect may depend on how far the owner has evolved.
    Varies,
    /// The write's effect depends only on state that changes at the owner's
    /// appointments or by accesses, so a write that changes nothing changes
    /// nothing when repeated before the next appointment.
    Settled,
    /// The write replaces a field wholly, and the field influences the rest
    /// of the owner only at its appointments: a later identical write leaves
    /// the owner as if the earlier one had not happened.
    Replaces,
}
/// The result of a CPU write.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Written {
    /// The owners whose state or gating changed and must be settled, or
    /// `None` when the write changed nothing beyond retained values.
    pub owners: Option<u16>,
    /// Repeating the write with nothing else changing, before the next
    /// appointment, leaves the board as performing it once does.
    pub recurs: bool,
}

pub(crate) const MEMORY_STATES: u8 = 2;
/// Register cells at 0xf000..=0xf0ff and 0xff80..=0xffff.
const REGISTER_CELLS: usize = 0x180;
#[inline]
pub(crate) const fn memory_target(a: u16) -> Option<Target> {
    if a < 0xc000 {
        Some(Target::Flash)
    } else if matches!(a, RAM_START..=0xff7f) {
        Some(Target::Ram)
    } else {
        None
    }
}

/// A port read: the analog input pins the converter selects read low.
fn gpio_value(gpio: &Gpio, adc: &Adc, a: u16) -> u8 {
    let mask = if a == 0xffde && (4..=9).contains(&adc.channel()) {
        1 << (adc.channel() - 4)
    } else {
        0
    };
    gpio.read(a) & !mask
}
/// A native word register's value; other targets have none.
fn word_value(
    access: Access,
    timer_w: &TimerW,
    adc: &Adc,
    aec: &Aec,
    clocks: &Clocks,
) -> Result<u16, Error> {
    if !access.native_word() {
        return Ok(0);
    }
    let a = access.address & !1;
    match access.target {
        Target::TimerW => timer_w.read_word(a, clocks),
        Target::Adc => Ok(adc.result()),
        Target::Aec => aec.read_word(a),
        Target::Ram => Err(Error::Internal("memory read through the register owners")),
        _ => Ok(0),
    }
}

/// A register access completed by its owner alone (see `Registers::local`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Local {
    pub value: u16,
    /// The access changed state, or observed state that can change before the
    /// next appointment (see `Transacted`).
    pub disturbs: bool,
    /// An interrupt request changed: whoever caches the offered request
    /// derives it again before the next instruction.
    pub requests: bool,
}

/// The clocked and register owners, borrowed apart from RAM and the flash
/// array, so a memory-only executor can reach them between its own accesses.
pub(crate) struct Registers<'a> {
    clocks: &'a Clocks,
    control: &'a mut Control,
    gpio: &'a mut Gpio,
    rtc: &'a mut Rtc,
    ssu: &'a mut Ssu,
    sci: &'a mut Sci,
    iic: &'a mut Iic,
    timer_b1: &'a mut TimerB1,
    timer_w: &'a mut TimerW,
    watchdog: &'a mut Watchdog,
    adc: &'a mut Adc,
    aec: &'a mut Aec,
    comparators: &'a mut Comparators,
    supplied: bool,
}
impl Registers<'_> {
    /// The serial engine's MCU parts (`machine::serial::Wires`).
    pub(crate) fn serial_parts(&mut self) -> (&mut Ssu, &mut Gpio, &mut Iic, &Clocks) {
        (self.ssu, self.gpio, self.iic, self.clocks)
    }
    /// When the SCI is lazy (`Sci::lazy`), the time before which no access
    /// sees its registers change; `Some(None)` when none is due.
    pub(crate) fn sci_flag_deadline(&self) -> Result<Option<Option<Time>>, Error> {
        if !self.sci.lazy(0) {
            return Ok(None);
        }
        Ok(Some(self.sci.flag_deadline(self.clocks)?))
    }
    /// Bring the clocked owners in `mask` up to `now`. Returns whether the
    /// watchdog overflowed into a reset. SSU, IIC and the startup sequence
    /// are not clock dependencies here (`Mcu::sync_peripherals_at`).
    pub fn sync(&mut self, mask: u16, now: Time) -> Result<bool, Error> {
        if mask & schedule::SCI != 0 {
            self.sci.sync(now, self.clocks)?;
        }
        if mask & schedule::ADC != 0 {
            self.adc.sync(now, self.clocks)?;
        }
        if mask & schedule::COMPARATORS != 0 {
            self.comparators.sync(now)?;
        }
        if mask & schedule::AEC != 0 {
            self.aec.sync(now, self.clocks)?;
            let requests = self.aec.take_requests();
            if requests & 1 != 0 {
                self.control.irr2 |= 1;
            }
            if requests & 2 != 0 {
                self.control.irr1 |= 4;
            }
        }
        if mask & schedule::RTC != 0 {
            self.rtc.sync(now, self.clocks)?;
        }
        if mask & schedule::TIMER_B1 != 0 && self.timer_b1.sync(now, self.clocks) {
            self.control.irr2 |= 4;
        }
        if mask & schedule::TIMER_W != 0 {
            self.timer_w.sync(now, self.clocks)?;
        }
        Ok(mask & schedule::WATCHDOG != 0 && self.watchdog.sync(now, self.clocks))
    }
    /// A register read, after its owners are synchronized. RAM, flash and
    /// flash control are the caller's.
    pub fn read(
        &mut self,
        access: Access,
        now: impl FnOnce() -> Result<Time, Error>,
    ) -> Result<u16, Error> {
        let a = access.address;
        if access.width == Width::Word {
            return self.word_value(access);
        }
        if access.native_word() {
            let word = self.word_value(access)?;
            return Ok(u16::from((word >> if a & 1 == 0 { 8 } else { 0 }) as u8));
        }
        let value = match access.target {
            Target::Gpio => gpio_value(self.gpio, self.adc, a),
            Target::Control => self.control.read(a),
            Target::Sci => self.sci.read(a),
            Target::Aec => self.aec.read(a),
            Target::Rtc => self.rtc.read(a),
            Target::Comparators => self.comparators.read(a),
            Target::TimerB1 => self.timer_b1.read(a),
            Target::Ssu => self.ssu.read_at(a, now, self.clocks)?,
            Target::TimerW => self.timer_w.read(a),
            Target::Watchdog => self.watchdog.read(a),
            Target::Adc => self.adc.peek(a),
            Target::Iic => self.iic.read_at(a, now, self.clocks)?,
            Target::Unmapped => 0,
            Target::Flash | Target::Ram | Target::FlashControl => {
                return Err(Error::Internal("memory read through the register owners"))
            }
        };
        Ok(u16::from(value))
    }
    fn word_value(&self, access: Access) -> Result<u16, Error> {
        word_value(access, self.timer_w, self.adc, self.aec, self.clocks)
    }
    /// The interrupt requests the owners in `mask` can change by being
    /// synchronized or accessed, folded together for comparison.
    fn requests(&self, mask: u16) -> u64 {
        let mut r = u64::from(self.control.irr1) | u64::from(self.control.irr2) << 8;
        if mask & schedule::RTC != 0 {
            r |= u64::from(self.rtc.interrupt().map_or(0, |v| v | 0x80)) << 16;
        }
        if mask & schedule::WATCHDOG != 0 {
            r |= u64::from(self.watchdog.interrupt()) << 24;
        }
        if mask & schedule::SSU != 0 {
            r |= u64::from(self.ssu.interrupt()) << 25;
        }
        if mask & schedule::TIMER_W != 0 {
            r |= u64::from(self.timer_w.interrupt()) << 26;
        }
        if mask & schedule::SCI != 0 {
            r |= u64::from(self.sci.interrupt()) << 27;
        }
        if mask & schedule::IIC != 0 {
            r |= u64::from(self.iic.interrupt()) << 28;
        }
        if mask & schedule::COMPARATORS != 0 {
            r |= u64::from(self.comparators.interrupt().map_or(0, |v| v | 0x80)) << 32;
        }
        r
    }
    /// Perform an access whose every consequence stays with its owners, as
    /// `Board::transact` would, or decline it with nothing changed (`None`).
    ///
    /// Every change that reaches another owner, the pins, the clocks, the
    /// serial network or the CPU's interrupt input is an appointment, and an
    /// access inside an interval precedes every appointment, so bringing the
    /// accessed owners up to the access is their own business. A read is local
    /// unless it starts something or observes the serial network. A write is
    /// local (the board brings the network up to every write only in case the
    /// write moves pins, which the writes accepted here never do) when performing it on a copy of its owner leaves the owner as it
    /// was, or makes one of the owner-local changes accepted below; anything
    /// else declines before the owner changes. `at` is the access's
    /// completion time.
    pub fn local(
        &mut self,
        access: Access,
        write: Option<(u16, WriteOrigin)>,
        at: impl FnOnce() -> Result<Time, Error>,
    ) -> Result<Option<Local>, Error> {
        if !self.supplied
            || access.memory()
            || (write.is_none() && access.observes_serial())
            || access.configures_board()
            || access.owners & schedule::STARTUP != 0
            || matches!(access.target, Target::FlashControl | Target::Iic)
            || (access.width == Width::Word && !access.native_word())
        {
            return Ok(None);
        }
        match write {
            None => self.local_read(access, at),
            Some((value, origin)) => self.local_write(access, value, origin, at),
        }
    }
    fn local_read(
        &mut self,
        access: Access,
        at: impl FnOnce() -> Result<Time, Error>,
    ) -> Result<Option<Local>, Error> {
        if access.read_changes_state() {
            return Ok(None);
        }
        if access.settled() {
            let value = self.read(access, || {
                Err(Error::Internal("settled read projected time"))
            })?;
            return Ok(Some(Local {
                value,
                disturbs: false,
                requests: false,
            }));
        }
        let before = self.requests(access.owners);
        if self.sync(access.owners, at()?)? {
            return Err(Error::Internal("watchdog overflow before its appointment"));
        }
        let value = self.read(access, || Err(Error::Internal("local read projected time")))?;
        Ok(Some(Local {
            value,
            disturbs: true,
            requests: before != self.requests(access.owners),
        }))
    }
    fn local_write(
        &mut self,
        access: Access,
        value: u16,
        origin: WriteOrigin,
        at: impl FnOnce() -> Result<Time, Error>,
    ) -> Result<Option<Local>, Error> {
        let a = access.address;
        if access.width != Width::Byte || access.native_word() {
            return Ok(None);
        }
        let v = value as u8;
        let mut done = Local {
            value,
            disturbs: false,
            requests: false,
        };
        // A write that provably changes nothing needs its owner neither
        // synchronized nor copied.
        if match access.target {
            Target::Sci => a == 0xff9c && self.sci.inert_status_write(v),
            Target::Watchdog => self.watchdog.inert_write(a, v, origin),
            _ => false,
        } {
            return Ok(Some(done));
        }
        if access.target == Target::Watchdog && a == 0xffb1 {
            return Ok(self
                .watchdog
                .write_control1_local(v, origin)
                .map(|changed| {
                    done.disturbs = changed;
                    done
                }));
        }
        let now = at()?;
        if access.target == Target::Watchdog
            && a == 0xffb3
            && self.watchdog.replace_count_local(v, now, self.clocks)
        {
            return Ok(Some(done));
        }
        let before = self.requests(access.owners);
        // Bringing the owners up to the access is local (see above), whether
        // or not the write itself then is.
        if self.sync(access.owners, now)? {
            return Err(Error::Internal("watchdog overflow before its appointment"));
        }
        done.requests = before != self.requests(access.owners);
        match access.target {
            Target::Ssu if a == 0xf0eb => {
                // While a frame shifts, or its load is already due, a new TDR
                // byte waits for it and moves no appointment; a change to an
                // enabled request the board settles.
                if !self.ssu.master() || !self.ssu.loads_later_on_transmit() {
                    return Ok(None);
                }
                let mut copy = self.ssu.clone();
                copy.write(a, v, origin.is_mov(), now, self.clocks)?;
                if copy.interrupt() != self.ssu.interrupt() || !copy.same_schedule(self.ssu) {
                    return Ok(None);
                }
                done.disturbs = copy != *self.ssu;
                *self.ssu = copy;
                Ok(Some(done))
            }
            Target::Sci => {
                let mut copy = self.sci.clone();
                copy.write(a, v, now, self.clocks)?;
                Ok((copy == *self.sci).then_some(done))
            }
            // A count no larger than the one reached only moves the overflow
            // appointment later.
            Target::Watchdog if a == 0xffb3 && v <= self.watchdog.peek(a) => {
                self.watchdog.write(a, v, origin, now, self.clocks)?;
                Ok(Some(done))
            }
            _ => Ok(None),
        }
    }
}

impl Mcu {
    /// The serial engine's MCU parts (`machine::serial::Wires`).
    pub(crate) fn serial_parts(&mut self) -> (&mut Ssu, &mut Gpio, &mut Iic, &Clocks) {
        (&mut self.ssu, &mut self.gpio, &mut self.iic, &self.clocks)
    }
    /// RAM, the flash array and every other owner, borrowed together.
    pub(crate) fn memory_parts(&mut self) -> (&mut [u8; RAM_SIZE], &Flash, Registers<'_>) {
        (
            &mut self.ram,
            &self.flash,
            Registers {
                clocks: &self.clocks,
                control: &mut self.control,
                gpio: &mut self.gpio,
                rtc: &mut self.rtc,
                ssu: &mut self.ssu,
                sci: &mut self.sci,
                timer_b1: &mut self.timer_b1,
                timer_w: &mut self.timer_w,
                iic: &mut self.iic,
                watchdog: &mut self.watchdog,
                adc: &mut self.adc,
                aec: &mut self.aec,
                comparators: &mut self.comparators,
                supplied: self.startup.supplied(),
            },
        )
    }
    #[inline(always)]
    pub(crate) fn read_ram(&self, a: u16, width: Width) -> u16 {
        let i = usize::from(a - RAM_START);
        if width == Width::Word {
            u16::from_be_bytes([self.ram[i], self.ram[i + 1]])
        } else {
            u16::from(self.ram[i])
        }
    }
    #[inline(always)]
    /// Store to RAM; whether the stored bytes differ from the old ones.
    pub(crate) fn write_ram(&mut self, a: u16, width: Width, value: u16) -> bool {
        let i = usize::from(a - RAM_START);
        if width == Width::Word {
            let bytes = value.to_be_bytes();
            let changed = self.ram[i..i + 2] != bytes;
            self.ram[i..i + 2].copy_from_slice(&bytes);
            changed
        } else {
            let changed = self.ram[i] != value as u8;
            self.ram[i] = value as u8;
            changed
        }
    }
    /// Route an access. Register banks are described once, by `describe`.
    #[inline(always)]
    pub(crate) fn classify(address: u16, width: Width, write: bool) -> Access {
        const REGISTERS: [Access; 4 * REGISTER_CELLS] = {
            let mut entries = [Mcu::describe(0xf000, Width::Byte, false); 4 * REGISTER_CELLS];
            let mut i = 0;
            while i < entries.len() {
                let cell = i / 4;
                let address = if cell < 0x100 {
                    0xf000 + cell as u16
                } else {
                    0xff80 + (cell - 0x100) as u16
                };
                let width = if i & 2 != 0 { Width::Word } else { Width::Byte };
                entries[i] = Mcu::describe(address, width, i & 1 != 0);
                i += 1;
            }
            entries
        };
        let cell = match address {
            0xf000..=0xf0ff => usize::from(address - 0xf000),
            0xff80..=0xffff => 0x100 + usize::from(address - 0xff80),
            _ => return Self::describe(address, width, write),
        };
        REGISTERS[cell * 4 + (width as usize) * 2 + usize::from(write)]
    }
    #[cold]
    #[inline(never)]
    const fn describe(address: u16, width: Width, write: bool) -> Access {
        let a = if matches!(width, Width::Word) {
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
            // Port 3 shows TXD, which a late SCI moves by itself.
            Target::Gpio if a == 0xffd6 && !write => schedule::SCI,
            Target::Unmapped if matches!(a, 0xf0e0..=0xf0eb) => schedule::SSU,
            _ => 0,
        };
        let shared_clocks = write
            && (Control::clock_register(a)
                || matches!(a, 0xf06f | 0xf0d0 | 0xf0e2 | 0xf0f1 | 0xffbe | 0xf022)
                || Aec::handles(a));
        // SCI data and status only reach the pins through the SCI's own
        // drivers, which a peripheral change of the SCI re-resolves.
        let configuration = matches!(target, Target::Gpio | Target::Control | Target::Comparators)
            || (matches!(target, Target::Sci) && !matches!(a, 0xff9b..=0xff9d))
            || Aec::handles(a)
            || matches!(a, 0xffbe | 0xffbf);
        Access {
            address: a,
            width,
            target,
            owners: if shared_clocks { schedule::ALL } else { owners },
            states: Self::access_states(a, width) as u8,
            flags: ((Self::native_word(a & !1)) as u8)
                | (((configuration) as u8) << 1)
                | (((Self::read_changes_state(a)) as u8) << 2)
                | (((write
                    || match target {
                        Target::Gpio => Gpio::reads_serial(a),
                        Target::Ssu => Ssu::reads_shift(a),
                        Target::Iic => Self::read_changes_state(a),
                        _ => false,
                    }) as u8)
                    << 3)
                | (((!write
                    && match target {
                        Target::Ssu => Ssu::settled_read(a),
                        Target::Sci => Sci::settled_read(a),
                        _ => false,
                    }) as u8)
                    << 4),
        }
    }
    pub const fn is_memory(a: u16) -> bool {
        a < 0xc000 || matches!(a, RAM_START..=0xff7f)
    }
    pub const fn native_word(a: u16) -> bool {
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
    pub const fn access_states(a: u16, _width: Width) -> u64 {
        if matches!(a, 0xf0e0..=0xf0e4 | 0xf0e9 | 0xf0eb | 0xff98..=0xff9d | 0xffa6) {
            3
        } else {
            2
        }
    }
    pub(crate) const fn read_changes_state(a: u16) -> bool {
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
    pub(crate) fn read_access_at(
        &mut self,
        access: Access,
        now: impl FnOnce() -> Result<Time, Error>,
        out: &mut dyn Output,
    ) -> Result<u16, Error> {
        let a = access.address;
        if access.width == Width::Word {
            return match access.target {
                Target::Flash => self.flash.read16(a, now()?, out),
                Target::Ram => {
                    let i = usize::from((a & !1) - RAM_START);
                    Ok(u16::from_be_bytes([self.ram[i], self.ram[i + 1]]))
                }
                _ => self.memory_parts().2.read(access, now),
            };
        }
        match access.target {
            Target::Flash => Ok(u16::from(self.flash.read8(a, now()?, out)?)),
            Target::Ram => Ok(u16::from(self.ram[usize::from(a - RAM_START)])),
            Target::FlashControl => Ok(u16::from(self.flash.register(a))),
            _ => self.memory_parts().2.read(access, now),
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
    /// Perform a CPU write.
    pub(crate) fn write_access(
        &mut self,
        access: Access,
        value: u16,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<Written, Error> {
        let varies = |owners| Written {
            owners,
            recurs: false,
        };
        if access.width == Width::Word {
            self.write_word(access, value, now, out)?;
            return Ok(varies(Some(access.owners)));
        }
        let recurrence = match access.target {
            Target::Sci => Sci::recurrence(access.address),
            Target::Watchdog => Watchdog::recurrence(access.address),
            _ => Recurrence::Varies,
        };
        match recurrence {
            Recurrence::Varies => self
                .write_varying(access, value, origin, now, out)
                .map(varies),
            Recurrence::Replaces => Ok(Written {
                owners: self.write_varying(access, value, origin, now, out)?,
                recurs: true,
            }),
            Recurrence::Settled => {
                let (owners, unchanged) = match access.target {
                    Target::Sci => {
                        self.write_observed(|m| &m.sci, access, value, origin, now, out)?
                    }
                    Target::Watchdog => {
                        self.write_observed(|m| &m.watchdog, access, value, origin, now, out)?
                    }
                    _ => return Err(Error::Internal("settled write to an unobserved owner")),
                };
                // A write that changed nothing has nothing to settle.
                Ok(Written {
                    owners: owners.filter(|_| !unchanged),
                    recurs: unchanged,
                })
            }
        }
    }
    /// Perform a write, and report whether it left `owner` unchanged.
    fn write_observed<T: Clone + PartialEq>(
        &mut self,
        owner: fn(&Self) -> &T,
        access: Access,
        value: u16,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(Option<u16>, bool), Error> {
        let before = owner(self).clone();
        let owners = self.write_varying(access, value, origin, now, out)?;
        Ok((owners, before == *owner(self)))
    }
    fn write_varying(
        &mut self,
        access: Access,
        value: u16,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<Option<u16>, Error> {
        let a = access.address;
        let v = value as u8;
        if access.target == Target::Gpio {
            return Ok(self.write_gpio(a, v)?.then_some(access.owners));
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
            let owners = self.write_byte(access, v, origin, now, out)?;
            self.admission_enables[index] |= before & !self.peek_access(access)? & mask;
            Ok(Some(owners))
        } else {
            self.write_byte(access, v, origin, now, out).map(Some)
        }
    }
    fn write_byte(
        &mut self,
        access: Access,
        v: u8,
        origin: WriteOrigin,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<u16, Error> {
        let a = access.address;
        // Native word latches ignore byte stores rather than synthesize an
        // unspecified read-modify-write of the opposite lane.
        if access.native_word() && !access.memory() {
            return Ok(access.owners);
        }
        if access.target == Target::Watchdog {
            let gating = self.watchdog.gating();
            self.watchdog.write(a, v, origin, now, &self.clocks)?;
            if self.watchdog.gating() == gating {
                return Ok(access.owners);
            }
            // A clock-source change: every other owner settles the interval
            // it counted under the old gating before the gates move.
            if self.sync_peripherals(schedule::ALL, now, out)? {
                return Err(Error::Internal("watchdog overflow at its own write"));
            }
            self.apply_gates(now, out)?;
            return Ok(schedule::ALL);
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
        .map(|()| access.owners)
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
            let word = word_value(access, &self.timer_w, &self.adc, &self.aec, &self.clocks)?;
            return Ok((word >> if a & 1 == 0 { 8 } else { 0 }) as u8);
        }
        Ok(match access.target {
            Target::Flash => self.flash.settled_byte(a),
            Target::Ram => self.ram[usize::from(a - RAM_START)],
            Target::Gpio => gpio_value(&self.gpio, &self.adc, a),
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
