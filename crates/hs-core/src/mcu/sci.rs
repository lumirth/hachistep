//! SCI3 shift/holding registers, sampled reception, SCK, and the IrDA codec.
//! The owner drives electrical pins. The board separately resolves transceiver
//! shutdown and converts its actual transmit pin into optical output.
mod baud;
mod frame;
#[cfg(test)]
mod tests;

use super::clocks::{ClockWait, Clocks, Tap};
use crate::{
    error::Error,
    signals::Drive,
    time::{Duration, Time, TimeError},
};
use baud::Baud;
use frame::Format;

#[derive(
    borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct Pins {
    pub clock: Option<Drive>,
    pub transmit: Option<bool>,
    pub receive: bool,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct Character {
    format: Format,
    word: u16,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Tx {
    Blocked {
        character: Character,
    } = 0,
    Mark {
        next: u64,
    } = 1,
    Start {
        character: Character,
        next: u64,
    } = 2,
    Data {
        character: Character,
        cell: u8,
        next: Option<u64>,
    } = 3,
    /// TSR can already hold the next character while the old stop/D7 remains
    /// on TXD. A later TDR write must not overwrite that preloaded character.
    Tail {
        loaded: Option<Character>,
        next: Option<u64>,
    } = 4,
}
impl Tx {
    fn next(self) -> Option<u64> {
        match self {
            Self::Blocked { .. } => None,
            Self::Mark { next } | Self::Start { next, .. } => Some(next),
            Self::Data { next, .. } | Self::Tail { next, .. } => next,
        }
    }
    fn synchronous(self) -> bool {
        matches!(
            self,
            Self::Data { next: None, .. } | Self::Tail { next: None, .. }
        )
    }
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct Rx {
    format: Format,
    value: u8,
    position: i8,
    parity: bool,
    error: u8,
    next: Option<u64>,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum PulseEnd {
    Basic(u64) = 0,
    Phi(ClockWait) = 1,
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Sci {
    spcr: u8,
    smr: u8,
    brr: u8,
    scr: u8,
    tdr: u8,
    ssr: u8,
    rdr: u8,
    semr: u8,
    ircr: u8,
    seen: u8,
    module: bool,
    main: bool,
    sub: bool,
    reset_mode: bool,
    baud: Baud,
    holding: Option<u8>,
    tx: Option<Tx>,
    rx: Option<Rx>,
    uart: bool,
    infrared: bool,
    bit_start: u64,
    pulse_start: Option<u64>,
    pulse_end: Option<PulseEnd>,
    receive_pulse: Option<u64>,
    input: bool,
    input_clock: bool,
    sck: bool,
    sync_next: Option<u64>,
    mux_glitch: Option<Time>,
    #[borsh(skip)]
    pub transmitted: u64,
    #[borsh(skip)]
    pub received: u64,
}
impl Default for Sci {
    fn default() -> Self {
        Self {
            spcr: 0xc0,
            smr: 0,
            brr: 255,
            scr: 0,
            tdr: 255,
            ssr: 0x84,
            rdr: 0,
            semr: 0,
            ircr: 0,
            seen: 0,
            module: false,
            main: false,
            sub: false,
            reset_mode: false,
            baud: Baud::default(),
            holding: None,
            tx: None,
            rx: None,
            uart: true,
            infrared: false,
            bit_start: 0,
            pulse_start: None,
            pulse_end: None,
            receive_pulse: None,
            input: true,
            input_clock: true,
            sck: true,
            sync_next: None,
            mux_glitch: None,
            transmitted: 0,
            received: 0,
        }
    }
}
impl Sci {
    pub(crate) fn supply_lost(&mut self) {
        // A mux's transient electrical excursion cannot outlive its supply.
        self.mux_glitch = None;
    }
    pub fn handles(a: u16) -> bool {
        matches!(a, 0xff91 | 0xff98..=0xff9d | 0xffa6 | 0xffa7)
    }
    fn reload(&self) -> u16 {
        u16::from(self.brr) + 1
    }
    fn format(&self) -> Format {
        Format::new(self.smr, self.semr)
    }
    fn external(&self) -> bool {
        self.scr & 2 != 0
    }
    fn clock_output(&self) -> bool {
        !self.external() && (self.scr & 1 != 0 || self.smr & 0x80 != 0)
    }
    fn running(&self) -> bool {
        self.module && !self.reset_mode && (self.main || (self.sub && self.smr & 3 == 1))
    }
    fn tap(&self) -> Tap {
        match self.smr & 3 {
            0 => Tap::system(1),
            1 => Tap::watch(1),
            2 => Tap::system(16),
            _ => Tap::system(64),
        }
    }
    pub fn sync(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        self.baud.sync(now, c, self.reload())
    }
    fn select_clock(&mut self, now: Time, c: &Clocks) {
        self.baud
            .select(now, c, self.tap(), self.running(), self.external());
    }
    fn reset_registers(&mut self, retain_spcr: bool) {
        let (spcr, input, clock, tx, rx) = (
            self.spcr,
            self.input,
            self.input_clock,
            self.transmitted,
            self.received,
        );
        *self = Self::default();
        if retain_spcr {
            self.spcr = spcr;
        }
        self.input = input;
        self.input_clock = clock;
        self.transmitted = tx;
        self.received = rx;
    }
    pub fn set_power(
        &mut self,
        module: bool,
        main: bool,
        sub: bool,
        reset_mode: bool,
        now: Time,
        c: &Clocks,
    ) -> Result<(), Error> {
        self.sync(now, c)?;
        if !module && self.module {
            self.reset_registers(false);
        } else if reset_mode && !self.reset_mode {
            self.reset_registers(true);
        }
        if let Some(PulseEnd::Phi(ref mut wait)) = self.pulse_end {
            if main {
                wait.resume(now, c)?;
            } else {
                wait.pause(now, c)?;
            }
        }
        self.module = module;
        self.main = main;
        self.sub = sub;
        self.reset_mode = reset_mode;
        self.select_clock(now, c);
        self.refresh_sync_clock();
        Ok(())
    }
    pub fn pins(&self) -> Pins {
        let clock = if self.mux_glitch.is_some() {
            Some(Drive::Low)
        } else if self.external() {
            Some(Drive::Floating)
        } else if self.clock_output() {
            let high = if self.smr & 0x80 != 0 {
                self.sck
            } else {
                self.baud.half % self.format().half_bit >= self.format().half_bit / 2
            };
            Some(if high { Drive::High } else { Drive::Low })
        } else {
            None
        };
        Pins {
            clock,
            transmit: (self.spcr & 0x10 != 0).then_some(
                (if self.ircr & 0x80 != 0 {
                    self.infrared
                } else {
                    self.uart
                }) ^ (self.spcr & 2 != 0),
            ),
            receive: self.scr & 0x10 != 0,
        }
    }
    pub fn deadline(&self, c: &Clocks) -> Result<Option<Time>, Error> {
        let async_clock = (self.clock_output() && self.smr & 0x80 == 0).then(|| {
            let interval = self.format().half_bit / 2;
            self.baud.half + interval - self.baud.half % interval
        });
        let next = [
            self.tx.and_then(Tx::next),
            self.rx.and_then(|r| r.next),
            self.pulse_start,
            self.receive_pulse,
            self.sync_next,
            async_clock,
            match self.pulse_end {
                Some(PulseEnd::Basic(n)) => Some(n),
                _ => None,
            },
        ]
        .into_iter()
        .flatten()
        .min();
        let basic = next
            .map(|n| self.baud.deadline(n, c, self.reload()))
            .transpose()?
            .flatten();
        let pulse = match self.pulse_end {
            Some(PulseEnd::Phi(wait)) => wait.deadline(c)?,
            _ => None,
        };
        Ok([basic, pulse, self.mux_glitch].into_iter().flatten().min())
    }
    pub fn interrupt(&self) -> bool {
        self.interrupt_with_enable(0)
    }
    pub(crate) fn interrupt_with_enable(&self, retained: u8) -> bool {
        let enable = self.scr | retained;
        self.module
            && ((self.ssr & 0x80 != 0 && enable & 0x80 != 0)
                || (self.ssr & 0x78 != 0 && enable & 0x40 != 0)
                || (self.ssr & 4 != 0 && enable & 4 != 0))
    }
    pub fn peek(&self, a: u16) -> u8 {
        match a {
            0xff91 => self.spcr,
            0xff98 => self.smr,
            0xff99 => self.brr,
            0xff9a => self.scr,
            0xff9b => self.tdr,
            0xff9c => self.ssr,
            0xff9d => self.rdr,
            0xffa6 => self.semr,
            _ => self.ircr,
        }
    }
    pub fn read(&mut self, a: u16) -> u8 {
        let value = self.peek(a);
        if a == 0xff9c {
            self.seen = value;
        }
        if a == 0xff9d {
            self.ssr &= !0x40;
            self.seen &= !0x40;
        }
        value
    }
    pub fn write(&mut self, a: u16, v: u8, now: Time, c: &Clocks) -> Result<(), Error> {
        if !self.module || self.reset_mode {
            return Ok(());
        }
        self.sync(now, c)?;
        let old_external = self.external();
        let old_sync_output = self.clock_output() && self.smr & 0x80 != 0;
        let old_ir_input = self.input ^ (self.spcr & 1 != 0);
        let old_ir = self.ircr & 0x80 != 0;
        match a {
            0xff91 => self.spcr = (v & 0x13) | 0xc0,
            0xff98 => self.smr = v,
            0xff99 => {
                self.brr = v;
                // Initialization permits waiting just ONE new bit period.
                // An idle write must not retain a previous 256-count BRC tail.
                if self.scr & 0x30 == 0 {
                    self.baud.reload(self.reload());
                }
            }
            0xffa6 => self.semr = v & 8,
            0xffa7 => self.ircr = v & 0xf0,
            0xff9a => {
                let old = self.scr;
                self.scr = v & 0xf7;
                if v & 0x20 == 0 {
                    self.tx = None;
                    self.holding = None;
                    self.pulse_start = None;
                    self.pulse_end = None;
                    self.uart = true;
                    self.infrared = false;
                    self.ssr |= 0x84;
                } else if old & 0x20 == 0 && !self.format().synchronous {
                    let f = self.format();
                    let start = self.baud.boundary(f.half_bit, now);
                    self.tx = Some(Tx::Mark {
                        next: start + u64::from(f.cells()) * f.half_bit,
                    });
                }
                if v & 0x10 == 0 {
                    self.rx = None;
                    self.receive_pulse = None;
                }
            }
            0xff9b => {
                self.tdr = v;
                if self.scr & 0x20 != 0 {
                    self.holding = Some(v);
                    self.ssr &= !0x84;
                    self.seen &= !0x84;
                }
            }
            0xff9c => {
                self.ssr &= !(self.seen & !v & 0xf8);
                self.seen &= v;
                if self.scr & 0x20 == 0 {
                    self.ssr |= 0x80;
                } else if self.ssr & 0x80 == 0 && self.holding.is_none() {
                    self.holding = Some(self.tdr);
                    self.ssr &= !4;
                }
            }
            0xff9d => {}
            _ => {
                return Err(Error::Unmapped {
                    address: a,
                    write: true,
                    width: 1,
                })
            }
        }
        if old_external != self.external() && self.external() {
            self.baud.connect(self.input_clock);
            self.sync_next = None;
        }
        if old_sync_output && !self.clock_output() && !self.external() {
            // A CPU mux write cannot be followed by another clock-control
            // access inside this half-phi propagation interval.
            let (n, d) = c.system_rate();
            let delay = Duration::from_raw((u128::from(d) << 63) / u128::from(n));
            self.mux_glitch = Some(now.checked_add(delay).ok_or(TimeError::Overflow)?);
        }
        self.select_clock(now, c);
        if old_ir && self.ircr & 0x80 == 0 {
            self.pulse_start = None;
            self.pulse_end = None;
            self.infrared = false;
            self.receive_pulse = None;
        } else if !old_ir
            && self.ircr & 0x80 != 0
            && !self.uart
            && self.bit_start + 13 >= self.baud.half
        {
            self.pulse_start = Some(self.bit_start + 13);
        }
        self.receive_change(old_ir && old_ir_input);
        if let Some(Tx::Tail { loaded: None, next }) = self.tx {
            if self.holding.is_some() {
                let loaded = self.preload();
                self.tx = Some(Tx::Tail { loaded, next });
            }
        }
        self.try_start(now)?;
        self.arm_receive();
        self.refresh_sync_clock();
        Ok(())
    }
    fn character(&self, value: u8) -> Character {
        let format = self.format();
        Character {
            format,
            word: format.word(value),
        }
    }
    fn preload(&mut self) -> Option<Character> {
        if let Some(value) = self.holding.take() {
            self.ssr |= 0x80;
            self.ssr &= !4;
            Some(self.character(value))
        } else {
            self.ssr |= 4;
            None
        }
    }
    fn drive(&mut self, high: bool) {
        self.uart = high;
        self.bit_start = self.baud.half;
        self.pulse_start = (self.ircr & 0x80 != 0 && !high).then_some(self.baud.half + 13);
    }
    fn begin(&mut self, character: Character) {
        let next = if character.format.synchronous {
            None
        } else {
            self.drive(character.word & 1 != 0);
            Some(self.baud.half + character.format.half_bit)
        };
        self.tx = Some(Tx::Data {
            character,
            cell: 0,
            next,
        });
    }
    fn try_start(&mut self, now: Time) -> Result<(), Error> {
        if let Some(Tx::Blocked { character }) = self.tx {
            if self.ssr & 0x38 == 0 {
                self.begin(character);
            }
            return Ok(());
        }
        if self.tx.is_some()
            || self.scr & 0x20 == 0
            || self.holding.is_none()
            || (self.format().synchronous && self.ssr & 0x38 != 0)
        {
            return Ok(());
        }
        let character = self
            .preload()
            .ok_or(Error::Internal("SCI holding register disappeared"))?;
        if character.format.synchronous {
            self.begin(character);
        } else {
            let next = self.baud.boundary(character.format.half_bit, now);
            if next == self.baud.half {
                self.begin(character);
            } else {
                self.tx = Some(Tx::Start { character, next });
            }
        }
        Ok(())
    }
    fn finish_tail(&mut self, loaded: Option<Character>, now: Time) -> Result<(), Error> {
        self.transmitted = self.transmitted.wrapping_add(1);
        self.tx = None;
        if let Some(character) = loaded {
            if character.format.synchronous && self.ssr & 0x38 != 0 {
                self.tx = Some(Tx::Blocked { character });
            } else {
                self.begin(character);
            }
        } else {
            self.try_start(now)?;
        }
        Ok(())
    }
    fn advance_tx(&mut self, now: Time) -> Result<(), Error> {
        let Some(tx) = self.tx else {
            return Ok(());
        };
        if tx.next() != Some(self.baud.half) {
            return Ok(());
        }
        match tx {
            Tx::Blocked { .. } => {}
            Tx::Mark { .. } => {
                self.tx = None;
                self.try_start(now)?;
            }
            Tx::Start { character, .. } => self.begin(character),
            Tx::Data {
                character, cell, ..
            } => {
                let cell = cell + 1;
                self.drive(character.word & (1 << cell) != 0);
                if cell == character.format.stop() {
                    let loaded = self.preload();
                    let next = Some(
                        self.baud.half
                            + u64::from(character.format.stops) * character.format.half_bit,
                    );
                    self.tx = Some(Tx::Tail { loaded, next });
                } else {
                    self.tx = Some(Tx::Data {
                        character,
                        cell,
                        next: Some(self.baud.half + character.format.half_bit),
                    });
                }
            }
            Tx::Tail { loaded, .. } => self.finish_tail(loaded, now)?,
        }
        Ok(())
    }
    fn input_level(&self) -> bool {
        let physical = self.input ^ (self.spcr & 1 != 0);
        if self.ircr & 0x80 != 0 {
            !(physical || self.receive_pulse.is_some())
        } else {
            physical
        }
    }
    fn receive_change(&mut self, was_positive: bool) {
        let positive = self.input ^ (self.spcr & 1 != 0);
        if self.ircr & 0x80 != 0 && positive && !was_positive && self.scr & 0x10 != 0 {
            self.receive_pulse = Some(self.baud.half + self.format().half_bit);
        }
    }
    fn arm_receive(&mut self) {
        if self.scr & 0x10 != 0
            && self.ssr & 0x38 == 0
            && self.rx.is_none()
            && !self.format().synchronous
            && !self.input_level()
        {
            self.rx = Some(Rx {
                format: self.format(),
                value: 0,
                position: -2,
                parity: false,
                error: 0,
                next: Some(self.baud.next_fall()),
            });
        }
    }
    fn complete_receive(&mut self, rx: Rx) {
        self.rx = None;
        if self.ssr & 0x40 != 0 {
            self.ssr |= rx.error | 0x20;
        } else {
            self.rdr = rx.value;
            self.ssr |= rx.error;
            if rx.error == 0 {
                self.ssr |= 0x40;
                self.received = self.received.wrapping_add(1);
            }
        }
    }
    fn advance_rx(&mut self) {
        let Some(mut rx) = self.rx else {
            return;
        };
        if rx.next != Some(self.baud.half) {
            return;
        }
        let high = self.input_level();
        match rx.position {
            -2 => {
                if high || self.format().synchronous {
                    self.rx = None;
                    return;
                }
                rx.format = self.format();
                rx.position = -1;
                rx.next = Some(self.baud.half + rx.format.half_bit / 2 - 1);
                self.rx = Some(rx);
                return;
            }
            -1 if high => {
                self.rx = None;
                return;
            }
            -1 => {}
            n if n < rx.format.data as i8 => {
                if high {
                    rx.value |= 1 << n;
                    rx.parity = !rx.parity;
                }
            }
            n if n == rx.format.data as i8 && rx.format.parity => {
                if high != (rx.parity ^ rx.format.odd) {
                    rx.error |= 8;
                }
            }
            _ => {
                if !high {
                    rx.error |= 0x10;
                }
                self.complete_receive(rx);
                return;
            }
        }
        rx.position += 1;
        rx.next = Some(self.baud.half + rx.format.half_bit);
        self.rx = Some(rx);
    }
    fn sync_needed(&self) -> bool {
        self.tx.is_some_and(Tx::synchronous)
            || (self.scr & 0x10 != 0
                && self.ssr & 0x38 == 0
                && (self.format().synchronous || self.rx.is_some_and(|r| r.format.synchronous)))
    }
    fn refresh_sync_clock(&mut self) {
        if self.external() {
            self.sync_next = None;
        } else if self.sync_needed() {
            if self.sync_next.is_none() {
                self.sync_next = Some(self.baud.half + 2);
            }
        } else {
            self.sync_next = None;
            self.sck = true;
        }
    }
    fn sync_edge(&mut self, high: bool, now: Time) -> Result<(), Error> {
        if high {
            if self.scr & 0x10 != 0 && self.ssr & 0x38 == 0 {
                let initial = self.format();
                let rx = self.rx.or_else(|| {
                    initial.synchronous.then_some(Rx {
                        format: initial,
                        value: 0,
                        position: 0,
                        parity: false,
                        error: 0,
                        next: None,
                    })
                });
                if let Some(mut rx) = rx.filter(|r| r.format.synchronous) {
                    if self.input_level() {
                        rx.value |= 1 << rx.position;
                    }
                    rx.position += 1;
                    if rx.position == 8 {
                        self.complete_receive(rx);
                    } else {
                        self.rx = Some(rx);
                    }
                }
            }
            if let Some(Tx::Tail { loaded, next: None }) = self.tx {
                self.finish_tail(loaded, now)?;
            }
        } else if let Some(Tx::Data {
            character,
            cell,
            next: None,
        }) = self.tx
        {
            self.drive(character.word & (1 << cell) != 0);
            if cell == 7 {
                let loaded = self.preload();
                self.tx = Some(Tx::Tail { loaded, next: None });
            } else {
                self.tx = Some(Tx::Data {
                    character,
                    cell: cell + 1,
                    next: None,
                });
            }
        }
        Ok(())
    }
    /// P30 and P31 after GPIO/peripheral muxes, board shutdown, and fixture
    /// drivers have been resolved. Input inversion is local to the SCI.
    pub fn input_pins(
        &mut self,
        clock: Option<bool>,
        input: bool,
        now: Time,
        c: &Clocks,
    ) -> Result<(), Error> {
        self.sync(now, c)?;
        let was_positive = self.input ^ (self.spcr & 1 != 0);
        self.input = input;
        self.receive_change(was_positive);
        self.arm_receive();
        if let Some(high) = clock {
            self.input_clock = high;
            if self.baud.external_edge(now, high)? {
                self.process_basic(now, c)?;
                self.sync_edge(high, now)?;
                self.refresh_sync_clock();
            }
        }
        Ok(())
    }
    fn process_basic(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        if self.receive_pulse == Some(self.baud.half) {
            self.receive_pulse = None;
        }
        if self.pulse_end == Some(PulseEnd::Basic(self.baud.half)) {
            self.infrared = false;
            self.pulse_end = None;
        }
        self.advance_tx(now)?;
        if self.pulse_start == Some(self.baud.half) {
            self.pulse_start = None;
            let width = self.ircr >> 4 & 7;
            self.pulse_end = match width {
                0 => Some(PulseEnd::Basic(self.baud.half + 6)),
                n @ 1..=4 => {
                    let mut wait = ClockWait::after(now, 1 << n, Tap::system(1), c)?;
                    if !self.main {
                        wait.pause(now, c)?;
                    }
                    Some(PulseEnd::Phi(wait))
                }
                _ => None,
            };
            self.infrared = self.pulse_end.is_some();
        }
        self.advance_rx();
        self.arm_receive();
        Ok(())
    }
    pub fn advance(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        self.sync(now, c)?;
        if self.mux_glitch == Some(now) {
            self.mux_glitch = None;
        }
        if let Some(PulseEnd::Phi(wait)) = self.pulse_end {
            if wait.deadline(c)? == Some(now) {
                self.pulse_end = None;
                self.infrared = false;
            }
        }
        self.process_basic(now, c)?;
        if self.sync_next == Some(self.baud.half) {
            self.sync_next = None;
            self.sck = !self.sck;
            self.sync_edge(self.sck, now)?;
        }
        self.refresh_sync_clock();
        Ok(())
    }
}

impl Sci {
    pub(crate) fn validate(&self, now: Time) -> Result<(), Error> {
        use crate::state::require;
        self.baud.validate(now)?;
        require(
            self.spcr & 0xc0 == 0xc0
                && self.spcr & !0xd3 == 0
                && self.scr & !0xf7 == 0
                && self.semr & !8 == 0
                && self.ircr & !0xf0 == 0
                && self.ssr & !0xfc == 0
                && self.seen & !0xfc == 0,
            "invalid SCI storage bits",
        )?;
        if let Some(tx) = self.tx {
            let ch = match tx {
                Tx::Blocked { character }
                | Tx::Start { character, .. }
                | Tx::Data { character, .. } => Some(character),
                Tx::Tail { loaded, .. } => loaded,
                Tx::Mark { .. } => None,
            };
            if let Some(ch) = ch {
                ch.format.validate()?;
                require(
                    ch.word
                        >> if ch.format.synchronous {
                            8
                        } else {
                            ch.format.cells()
                        }
                        == 0,
                    "invalid transmit word",
                )?;
            }
            if let Tx::Data {
                character,
                cell,
                next,
            } = tx
            {
                require(
                    if character.format.synchronous {
                        cell < 8 && next.is_none()
                    } else {
                        cell < character.format.stop() && next.is_some()
                    },
                    "invalid transmit position",
                )?;
            }
            require(
                tx.next().is_none_or(|n| n.checked_add(1024).is_some()),
                "invalid transmit interval",
            )?;
        }
        if let Some(rx) = self.rx {
            rx.format.validate()?;
            require(rx.error & !0x18 == 0, "invalid receiver error latch")?;
            require(
                if rx.format.synchronous {
                    (0..8).contains(&rx.position) && rx.next.is_none()
                } else {
                    (-2..=(rx.format.data + u8::from(rx.format.parity)) as i8)
                        .contains(&rx.position)
                        && rx.next.is_some()
                },
                "invalid receive position",
            )?;
            require(
                rx.next.is_none_or(|n| n.checked_add(1024).is_some()),
                "invalid receive interval",
            )?;
        }
        if let Some(PulseEnd::Phi(w)) = self.pulse_end {
            w.validate()?;
        }
        require(
            self.bit_start.checked_add(1024).is_some()
                && [
                    self.pulse_start,
                    self.receive_pulse,
                    self.sync_next,
                    match self.pulse_end {
                        Some(PulseEnd::Basic(n)) => Some(n),
                        _ => None,
                    },
                ]
                .into_iter()
                .flatten()
                .all(|n| n.checked_add(1024).is_some()),
            "invalid serial edge ordinal",
        )
    }
}
