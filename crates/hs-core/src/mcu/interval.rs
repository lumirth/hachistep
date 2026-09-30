//! MCU-owned bus view, valid until the enclosing interaction bound.
use super::*;
use crate::cpu::{
    execution::{Bus, IntervalBus, Stop},
    Action,
};
use crate::mcu::clocks::{CpuCursor, CpuWindow};

pub(crate) struct Interval<'a> {
    mcu: &'a mut Mcu,
    normal_flash: Option<flash::NormalRead>,
    deferred_serial: bool,
    interrupt: Option<u8>,
    expired_interrupt: Option<u8>,
    admission_boundaries: u8,
    window: CpuWindow,
    first_due: bool,
    overshoot: Option<u64>,
    reads: u64,
    writes: u64,
    committed: Option<bus::Committed>,
}
impl Mcu {
    /// Machine has settled connections at entry and supplies a strict earliest
    /// device/input/caller bound. SSU is the sole wire recurrence deferred
    /// within that interval; every other driver transition is an interaction.
    /// Local operations expose RAM stores, canonical observations and retained
    /// GPIO transactions. Actual GPIO changes return their committed reply
    /// before CPU continuation; flash/control/other owner writes remain fenced.
    pub(crate) fn interval(&mut self, window: CpuWindow, at: Time) -> Result<Interval<'_>, Error> {
        let interrupt = self.interrupt();
        let expired_interrupt = if self.admission_enables == [0; 10] {
            interrupt
        } else {
            self.interrupt_with_retained([0; 10])
        };
        let admission_boundaries = self
            .control
            .admission_boundaries()
            .max(u8::from(self.admission_enables != [0; 10]));
        let normal_flash = self.flash.normal_read(at);
        // IIC and driver-changing external consequences bound this interval.
        // Only SSU's quiet edges can remain deferred across that bound.
        let deferred_serial = self.ssu.deadline(&self.clocks)?.is_some();
        Ok(Interval {
            mcu: self,
            normal_flash,
            deferred_serial,
            interrupt,
            expired_interrupt,
            admission_boundaries,
            window,
            first_due: true,
            overshoot: None,
            reads: 0,
            writes: 0,
            committed: None,
        })
    }
}
impl Interval<'_> {
    pub fn can_serve(&self, action: Action) -> bool {
        match action {
            Action::Read { address, width, .. } => {
                let a = Self::base(address, width);
                match bus::memory_target(a) {
                    Some(bus::Target::Ram) => true,
                    Some(bus::Target::Flash) => self.normal_flash.is_some(),
                    _ => self.local_read(Mcu::classify(a, width, false)),
                }
            }
            Action::Write { address, width, .. } => {
                Self::ram_address(address, width)
                    || self.local_write(Mcu::classify(address, width, true))
            }
            Action::Idle(states) => states != 0,
            Action::Sleep => false,
        }
    }
    fn base(address: u16, width: Width) -> u16 {
        if width == Width::Word {
            address & !1
        } else {
            address
        }
    }
    fn ram_address(address: u16, width: Width) -> bool {
        matches!(
            bus::memory_target(Self::base(address, width)),
            Some(bus::Target::Ram)
        )
    }
    fn local_read(&self, access: bus::Access) -> bool {
        (access.width != Width::Word || access.native_word())
            && !access.read_changes_state()
            && !(access.observes_serial() && self.deferred_serial)
    }
    fn local_write(&self, access: bus::Access) -> bool {
        access.retained_write() && !(access.observes_serial() && self.deferred_serial)
    }
    pub fn take_commit(&mut self) -> Option<bus::Committed> {
        self.committed.take()
    }
    #[inline(never)]
    fn read_register(&mut self, address: u16, width: Width, fetch: bool) -> Result<u16, Stop> {
        let access = Mcu::classify(address, width, false);
        if !self.local_read(access) {
            return Err(Stop::Request(Action::Read {
                address,
                width,
                fetch,
            }));
        }
        self.advance(access.states(), || Action::Read {
            address,
            width,
            fetch,
        })?;
        if access.owners != 0 {
            if self
                .mcu
                .sync_peripherals_at(access.owners, || self.window.at(), &mut ())?
            {
                return Err(Stop::Reset);
            }
            debug_assert_eq!(self.interrupt, self.mcu.interrupt());
        }
        let value = self
            .mcu
            .read_access_at(access, || self.window.at(), &mut ())?;
        self.reads = self.reads.wrapping_add(1);
        Ok(value)
    }
    #[inline(never)]
    fn write_register(
        &mut self,
        address: u16,
        width: Width,
        value: u16,
        mov_byte: bool,
    ) -> Result<(), Stop> {
        let access = Mcu::classify(address, width, true);
        if !self.local_write(access) {
            return Err(Stop::Request(Action::Write {
                address,
                width,
                value,
                mov_byte,
            }));
        }
        self.advance(access.states(), || Action::Write {
            address,
            width,
            value,
            mov_byte,
        })?;
        // The admitted GPIO owner ignores instruction-specific write origin.
        // Other owners remain fenced and retain full CPU provenance in Machine.
        let effects = self.mcu.write_gpio(access.address, value as u8)?;
        self.writes = self.writes.wrapping_add(1);
        if effects {
            self.committed = Some(bus::Committed { access, value: 0 });
            return Err(Stop::CommittedOwner);
        }
        Ok(())
    }
    #[inline]
    fn advance(&mut self, states: u64, action: impl FnOnce() -> Action) -> Result<(), Stop> {
        if self.first_due {
            self.first_due = false;
            return Ok(());
        }
        if !self.window.advance(states).map_err(Stop::Core)? {
            self.overshoot = Some(states);
            return Err(Stop::Horizon(action()));
        }
        Ok(())
    }
    pub fn finish(self, cursor: &mut CpuCursor) -> Result<(Time, u64, u64), Error> {
        #[cfg(not(feature = "profile-work"))]
        let at = match self.overshoot {
            Some(states) => self.window.before(states)?,
            None => self.window.at()?,
        };
        #[cfg(feature = "profile-work")]
        let at = {
            let at = match self.overshoot {
                Some(states) => self.window.before(states),
                None => self.window.at(),
            };
            self.mcu
                .clocks
                .cpu_time_materializations
                .add(self.window.time_materializations.get());
            at?
        };
        #[cfg(feature = "profile-work")]
        self.mcu.clocks.cpu_time_materializations.add(1);
        self.window.finish(cursor)?;
        Ok((at, self.reads, self.writes))
    }
}
impl Bus for Interval<'_> {
    const RUN_AHEAD: bool = true;
    #[inline(always)]
    fn read(&mut self, address: u16, width: Width, fetch: bool) -> Result<u16, Stop> {
        let a = Self::base(address, width);
        let memory = bus::memory_target(a);
        if memory.is_none() {
            return self.read_register(address, width, fetch);
        }
        if memory == Some(bus::Target::Flash) && self.normal_flash.is_none() {
            return Err(Stop::Request(Action::Read {
                address,
                width,
                fetch,
            }));
        }
        self.advance(u64::from(bus::MEMORY_STATES), || Action::Read {
            address,
            width,
            fetch,
        })?;
        let value = if memory == Some(bus::Target::Flash) {
            self.mcu.flash.read_normal(
                self.normal_flash.expect("proved normal flash interval"),
                a,
                width,
            )
        } else {
            let i = usize::from(a - RAM_START);
            if width == Width::Word {
                u16::from_be_bytes([self.mcu.ram[i], self.mcu.ram[i + 1]])
            } else {
                u16::from(self.mcu.ram[i])
            }
        };
        self.reads = self.reads.wrapping_add(1);
        Ok(value)
    }
    #[inline(always)]
    fn write(
        &mut self,
        address: u16,
        width: Width,
        value: u16,
        mov_byte: bool,
    ) -> Result<(), Stop> {
        if !Self::ram_address(address, width) {
            return self.write_register(address, width, value, mov_byte);
        }
        self.advance(u64::from(bus::MEMORY_STATES), || Action::Write {
            address,
            width,
            value,
            mov_byte,
        })?;
        let i = usize::from(Self::base(address, width) - RAM_START);
        if width == Width::Word {
            let [hi, lo] = value.to_be_bytes();
            self.mcu.ram[i] = hi;
            self.mcu.ram[i + 1] = lo;
        } else {
            self.mcu.ram[i] = value as u8;
        }
        self.writes = self.writes.wrapping_add(1);
        Ok(())
    }
    #[inline]
    fn idle(&mut self, states: u32) -> Result<(), Stop> {
        if states == 0 {
            return Err(Stop::Request(Action::Idle(0)));
        }
        self.advance(u64::from(states), || Action::Idle(states))
    }
}
impl IntervalBus for Interval<'_> {
    fn interrupt(&self) -> Option<u8> {
        self.interrupt
    }
    fn instruction_boundary(&mut self) {
        // All transactions that can rearm the history leave this interval.
        // Once the owner-issued count reaches zero, no captured bits change.
        if self.admission_boundaries != 0 {
            self.mcu.instruction_boundary();
            self.admission_boundaries -= 1;
            self.interrupt = self.expired_interrupt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcu() -> Mcu {
        Mcu::new(
            &[0; FLASH_SIZE],
            Frequencies {
                main_hz: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn edge(m: &Mcu, n: u64) -> Time {
        m.clocks.edge(n, Tap::system(1)).unwrap()
    }

    #[test]
    fn local_admission_ages_low_pin_clear_protection_for_exactly_two_boundaries() {
        let mut m = mcu();
        m.control.irq_switch(0, true);
        for (boundary, expected) in [(1, 1), (2, 0)] {
            let at = edge(&m, boundary * 2);
            let mut cursor = CpuCursor::new(&m.clocks);
            let window = cursor.window(at, edge(&m, 20), &m.clocks).unwrap();
            let mut bus = m.interval(window, at).unwrap();
            bus.instruction_boundary();
            bus.finish(&mut cursor).unwrap();
            m.control.write(0xfff6, 0).unwrap();
            assert_eq!(m.control.irr1 & 1, expected);
        }
    }
    #[test]
    fn retained_serial_status_qualifies_flags_without_a_clock_observation() {
        let mut m = mcu();
        m.ssu
            .write(0xf0e3, 0x80, true, Time::ZERO, &m.clocks)
            .unwrap();
        m.iic.write(0xf078, 0x90, Time::ZERO, &m.clocks).unwrap();
        let no_clock = || Err(Error::Internal("retained observation consumed time"));
        assert!(!m
            .sync_peripherals_at(schedule::SSU | schedule::IIC, no_clock, &mut ())
            .unwrap());
        for (address, flags) in [(0xf0e4, 4), (0xf07c, 0x80)] {
            let access = Mcu::classify(address, Width::Byte, false);
            assert_eq!(m.read_access_at(access, no_clock, &mut ()).unwrap(), flags);
            m.write8(address, 0, WriteOrigin::Other, Time::ZERO, &mut ())
                .unwrap();
            assert_eq!(u16::from(m.peek8(address).unwrap()) & flags, 0);
        }
    }

    #[test]
    fn capture_reads_observe_the_visibility_edge_inside_an_interval() {
        let mut m = mcu();
        let c = &m.clocks;
        m.timer_w.set_gate(true, Time::ZERO, c).unwrap();
        m.timer_w.write_word(0xf0f8, 0x1234, Time::ZERO, c).unwrap();
        m.timer_w.write(0xf0f4, 0x8e, Time::ZERO, c).unwrap();
        m.timer_w.write(0xf0f0, 0x90, Time::ZERO, c).unwrap();
        m.timer_w
            .input_pins([Some(false), None, None, None, None], Time::ZERO, c)
            .unwrap();
        m.timer_w
            .input_pins([Some(true), None, None, None, None], edge(&m, 3), c)
            .unwrap();
        let at = edge(&m, 6);
        m.timer_w.sync(at, &m.clocks).unwrap();
        let end = edge(&m, 20);
        let mut cursor = CpuCursor::new(&m.clocks);
        let window = cursor.window(at, end, &m.clocks).unwrap();
        let mut bus = m.interval(window, at).unwrap();
        assert_eq!(bus.read(0xf0f8, Width::Word, false).ok(), Some(0x1234));
        assert_eq!(bus.read(0xf0f8, Width::Word, false).ok(), Some(5));
        assert_eq!(bus.interrupt(), None);
    }

    #[test]
    fn local_status_reads_qualify_the_canonical_clear_strobe() {
        let mut m = mcu();
        m.timer_w.set_gate(true, Time::ZERO, &m.clocks).unwrap();
        m.timer_w
            .write_word(0xf0f8, 2, Time::ZERO, &m.clocks)
            .unwrap();
        m.timer_w
            .write(0xf0f0, 0x80, Time::ZERO, &m.clocks)
            .unwrap();
        let at = edge(&m, 3);
        m.timer_w.sync(at, &m.clocks).unwrap();
        assert_eq!(m.timer_w.peek(0xf0f3) & 1, 1);
        let mut cursor = CpuCursor::new(&m.clocks);
        let window = cursor.window(at, edge(&m, 20), &m.clocks).unwrap();
        let mut bus = m.interval(window, at).unwrap();
        assert_eq!(
            bus.read(0xf0f3, Width::Byte, false).ok().map(|v| v & 1),
            Some(1)
        );
        bus.finish(&mut cursor).unwrap();
        m.timer_w.write(0xf0f3, 0, edge(&m, 4), &m.clocks).unwrap();
        assert_eq!(m.timer_w.peek(0xf0f3) & 1, 0);
    }
    #[test]
    fn gpio_transactions_use_masked_retained_state_and_report_only_real_changes() {
        let mut m = mcu();
        m.write_gpio(0xffe4, 7).unwrap();
        let mut cursor = CpuCursor::new(&m.clocks);
        let at = edge(&m, 2);
        let window = cursor.window(at, edge(&m, 20), &m.clocks).unwrap();
        let mut bus = m.interval(window, at).unwrap();
        assert!(matches!(
            bus.write(0xffd4, Width::Byte, 7, true),
            Err(Stop::CommittedOwner)
        ));
        let commit = bus.take_commit().unwrap();
        assert_eq!(commit.access.address, 0xffd4);
        assert_eq!(bus.finish(&mut cursor).unwrap(), (at, 0, 1));
        assert_eq!(m.gpio.read(0xffd4), 7);

        let at = edge(&m, 4);
        let window = cursor.window(at, edge(&m, 20), &m.clocks).unwrap();
        let mut bus = m.interval(window, at).unwrap();
        // Unconnected bits are masked by the canonical register write.
        assert!(bus.write(0xffd4, Width::Byte, 0xff, false).is_ok());
        assert!(bus.take_commit().is_none());
        assert!(matches!(
            bus.write(0xffd4, Width::Byte, 6, false),
            Err(Stop::CommittedOwner)
        ));
        assert_eq!(bus.finish(&mut cursor).unwrap().2, 2);
        assert_eq!(m.gpio.read(0xffd4), 6);
    }
}
