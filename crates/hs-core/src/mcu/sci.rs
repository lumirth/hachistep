//! SCI3 shift/holding registers, sampled reception, SCK, and the IrDA codec.
//! The owner drives electrical pins. The board separately resolves transceiver
//! shutdown and converts its actual transmit pin into optical output.
mod baud;
mod frame;

use super::bus::Recurrence;
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

/// The most transmit-pin changes between two of the board's observations of
/// a late SCI. Only the board starts a character (a TDR write is never
/// local), and it observes the SCI as it does, so between observations at
/// most the shifting character and the held one transmit; a character of
/// at most twelve cells drives at most one change per cell and two per
/// pulse.
const OPTICAL_LOG: usize = 64;
/// Host control and output, not state. While the run's sink accepts late
/// output (`Output::exact_stops`), steps that move only TXD are internal:
/// TXD reaches nothing but the board's optical output and port reads, so
/// its changes are logged here, in order, for the board to report when it
/// next observes the SCI (`Board::report_optical`).
#[derive(Clone, Debug)]
struct Optical {
    late: bool,
    len: u8,
    levels: u64,
    at: [Time; OPTICAL_LOG],
}
/// The most queued light edges (`Sci::queue_light`).
const LIGHT_QUEUE: usize = 64;
/// Incident light edges the board has handed over, not state. Light reaches
/// only RXD while the board's route holds (`Sci::set_light_route`), so each
/// edge is an input step of this SCI alone, performed by `sync` at its time
/// after every other step at that time; the board then sees the level the
/// SCI reached (`Sci::light_level`). The queue is empty whenever a run
/// returns.
#[derive(Clone, Debug)]
struct Light {
    at: [Time; LIGHT_QUEUE],
    levels: u64,
    head: u8,
    len: u8,
    /// The RXD level light leaves: `Some(shutdown)` reads `shutdown || !light`;
    /// `None`, light reaches nothing.
    route: Option<bool>,
    /// The last edge performed, and whether the board has yet to see it.
    level: bool,
    unseen: bool,
    /// While the board serves appointments at this time, edges at it wait:
    /// inputs follow the appointments at their time.
    hold: Option<Time>,
}
impl Default for Light {
    fn default() -> Self {
        Self {
            at: [Time::ZERO; LIGHT_QUEUE],
            levels: 0,
            head: 0,
            len: 0,
            route: None,
            level: false,
            unseen: false,
            hold: None,
        }
    }
}
impl PartialEq for Light {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Light {}
impl Light {
    fn front(&self) -> Option<(Time, bool)> {
        (self.len != 0).then(|| {
            let i = usize::from(self.head);
            (self.at[i], self.levels >> i & 1 != 0)
        })
    }
    fn pop(&mut self) {
        self.head = ((usize::from(self.head) + 1) % LIGHT_QUEUE) as u8;
        self.len -= 1;
    }
}
impl Default for Optical {
    fn default() -> Self {
        Self {
            late: false,
            len: 0,
            levels: 0,
            at: [Time::ZERO; OPTICAL_LOG],
        }
    }
}
impl PartialEq for Optical {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Optical {}

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
    optical: Optical,
    #[borsh(skip)]
    light: Light,
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
            optical: Optical::default(),
            light: Light::default(),
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
    pub const fn handles(a: u16) -> bool {
        matches!(a, 0xff91 | 0xff98..=0xff9d | 0xffa6 | 0xffa7)
    }
    /// Every register but RDR changes only at this owner's deadline or by
    /// writes; repeating an SSR read qualifies the same flags again.
    pub(crate) const fn settled_read(a: u16) -> bool {
        Self::handles(a) && a != 0xff9d
    }
    /// An SSR write clears flags qualified by earlier reads and may load the
    /// transmitter from the holding register: all retained state, so a write
    /// that changes nothing keeps changing nothing until the next deadline.
    pub(crate) fn recurrence(a: u16) -> Recurrence {
        if a == 0xff9c {
            Recurrence::Settled
        } else {
            Recurrence::Varies
        }
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
    /// Advance to `now`, performing the internal steps on the way: receive
    /// samples before a character completes, receive pulse expiry and IrDA
    /// data cells change nothing outside the SCI by themselves, so they are
    /// not appointments (see `deadline`).
    pub fn sync(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        let phi_pulse = self.optical.late && matches!(self.pulse_end, Some(PulseEnd::Phi(_)));
        let light = self.due_light(now);
        // Every step lies on a later half than the current one, the first
        // of which begins after `now`.
        if !phi_pulse && light.is_none() && self.baud.quiet_through(now, c) {
            return Ok(());
        }
        if phi_pulse || !self.baud.advancing(c) {
            return self.sync_timed(now, c);
        }
        // Count halves rather than find their times: a step needs the time
        // only to log TXD or start a wait.
        let reload = self.reload();
        let reach = self.baud.half_at(now, c, reload);
        // The half of the first due light edge, while it is first.
        let mut light_half = None;
        loop {
            let next = self.internal_half().filter(|half| *half <= reach);
            // Light follows every other step at its time.
            if let Some((at, level)) = self.due_light(now) {
                let half = *light_half.get_or_insert_with(|| self.baud.half_at(at, c, reload));
                if next.is_none_or(|next| next > half) {
                    self.baud.step_to(half, reload);
                    self.perform_light(level);
                    light_half = None;
                    continue;
                }
            }
            let Some(half) = next else {
                break;
            };
            let before = self.transmit_level();
            self.baud.step_to(half, reload);
            self.process_basic(None, c)?;
            self.refresh_sync_clock();
            if self.transmit_level() != before {
                let at = self.baud.time(c)?;
                self.log_transmit(before, at)?;
            }
            self.complete_pulse(reach, c)?;
        }
        self.baud.step_to(reach, reload);
        self.baud.time(c)?;
        self.expire_receive_pulse();
        Ok(())
    }
    /// Late, a transmit pulse that starts and ends by `reach`, with no
    /// transmit step between, is logged whole rather than stepped twice.
    fn complete_pulse(&mut self, reach: u64, c: &Clocks) -> Result<(), Error> {
        let Some(start) = self.pulse_start else {
            return Ok(());
        };
        let end = start + 6;
        if !self.optical.late
            || self.ircr & 0x70 != 0
            || self.pulse_end.is_some()
            || end > reach
            || self.tx.and_then(Tx::next).is_some_and(|next| next <= end)
        {
            return Ok(());
        }
        let reload = self.reload();
        let idle = self.transmit_level();
        self.pulse_start = None;
        self.infrared = true;
        let lit = self.transmit_level();
        if lit != idle {
            let time = |half| {
                self.baud
                    .deadline(half, c, reload)?
                    .ok_or(Error::Internal("SCI pulse off its basic clock"))
            };
            let (from, to) = (time(start)?, time(end)?);
            self.log_transmit(idle, from)?;
            self.infrared = false;
            self.log_transmit(lit, to)?;
        }
        self.infrared = false;
        Ok(())
    }
    /// `sync` by the times of its steps: while a pulse ends on the system
    /// clock, or the BRC does not count.
    fn sync_timed(&mut self, now: Time, c: &Clocks) -> Result<(), Error> {
        loop {
            let basic = match self.internal_half() {
                Some(half) => self
                    .baud
                    .deadline(half, c, self.reload())?
                    .map(|at| (half, at)),
                None => None,
            };
            let pulse_end = match self.pulse_end {
                Some(PulseEnd::Phi(wait)) if self.optical.late => wait.deadline(c)?,
                _ => None,
            };
            let before = self.transmit_level();
            // A pulse ends before a basic step at the same time (`advance`).
            if let Some(at) =
                pulse_end.filter(|at| *at <= now && basic.is_none_or(|(_, b)| *at <= b))
            {
                self.pulse_end = None;
                self.infrared = false;
                self.log_transmit(before, at)?;
                continue;
            }
            if let Some((at, level)) = self.due_light(now) {
                if basic.is_none_or(|(_, b)| b > at) {
                    self.baud.sync(at, c, self.reload())?;
                    self.perform_light(level);
                    continue;
                }
            }
            let Some((half, at)) = basic.filter(|(_, at)| *at <= now) else {
                break;
            };
            self.baud.sync(at, c, self.reload())?;
            if self.baud.half != half {
                return Err(Error::Internal("SCI internal step off its basic clock"));
            }
            self.process_basic(Some(at), c)?;
            self.refresh_sync_clock();
            self.log_transmit(before, at)?;
        }
        self.baud.sync(now, c, self.reload())?;
        self.expire_receive_pulse();
        Ok(())
    }
    fn perform_light(&mut self, level: bool) {
        self.light.pop();
        self.light.level = level;
        self.light.unseen = true;
        if let Some(shutdown) = self.light.route {
            self.receive_input(shutdown || !level);
        }
    }
    /// The next queued light edge `sync` performs on its way to `now`.
    fn due_light(&self, now: Time) -> Option<(Time, bool)> {
        self.light
            .front()
            .filter(|(at, _)| *at <= now && self.light.hold.is_none_or(|hold| *at < hold))
    }
    /// RXD now shows `level`, the caller having synchronized.
    fn receive_input(&mut self, level: bool) {
        if level == self.input {
            return;
        }
        let was_positive = self.input ^ (self.spcr & 1 != 0);
        self.input = level;
        self.receive_change(was_positive);
        self.arm_receive();
    }
    /// Queue a light edge after every queued one; false when the queue is full.
    pub(crate) fn queue_light(&mut self, at: Time, level: bool) -> bool {
        let light = &mut self.light;
        let len = usize::from(light.len);
        if len == LIGHT_QUEUE
            || light
                .front()
                .is_some_and(|_| light.at[(usize::from(light.head) + len - 1) % LIGHT_QUEUE] >= at)
        {
            return false;
        }
        let i = (usize::from(light.head) + len) % LIGHT_QUEUE;
        light.at[i] = at;
        light.levels = (light.levels & !(1 << i)) | (u64::from(level) << i);
        light.len += 1;
        true
    }
    /// How light reaches RXD from now on (`Light::route`).
    pub(crate) fn set_light_route(&mut self, route: Option<bool>) {
        self.light.route = route;
    }
    pub(crate) fn hold_light(&mut self, hold: Option<Time>) {
        self.light.hold = hold;
    }
    /// The level of the last light edge performed since the board last
    /// asked, if any.
    pub(crate) fn take_light_level(&mut self) -> Option<bool> {
        let light = &mut self.light;
        core::mem::take(&mut light.unseen).then_some(light.level)
    }
    /// Return the edges not yet performed; how many.
    pub(crate) fn unqueue_light(&mut self) -> usize {
        core::mem::take(&mut self.light.len).into()
    }
    /// The level TXD shows, while it is the SCI's.
    fn transmit_level(&self) -> Option<bool> {
        (self.spcr & 0x10 != 0).then_some(
            (if self.ircr & 0x80 != 0 {
                self.infrared
            } else {
                self.uart
            }) ^ (self.spcr & 2 != 0),
        )
    }
    fn log_transmit(&mut self, before: Option<bool>, at: Time) -> Result<(), Error> {
        let Some(level) = self.transmit_level().filter(|level| before != Some(*level)) else {
            return Ok(());
        };
        let log = &mut self.optical;
        let i = usize::from(log.len);
        if i == OPTICAL_LOG {
            return Err(Error::Internal("SCI optical log overflow"));
        }
        log.at[i] = at;
        log.levels = (log.levels & !(1 << i)) | (u64::from(level) << i);
        log.len += 1;
        Ok(())
    }
    /// Whether steps that move only TXD are internal (`Optical`).
    pub(crate) fn set_late(&mut self, late: bool, now: Time, c: &Clocks) -> Result<(), Error> {
        // Lazily pending steps before `now` precede any appointment.
        if self.lazy(0) && !late {
            if let Some(before) = now.raw().checked_sub(1) {
                self.sync(Time::from_raw(before), c)?;
            }
        }
        self.optical.late = late;
        Ok(())
    }
    /// Whether the board must bring this SCI up to an observation of its
    /// pins: it is late and may have moved TXD by itself.
    pub(crate) fn optical_pending(&self) -> bool {
        self.optical.len != 0
            || self.light.len != 0
            || self.light.unseen
            || (self.optical.late
                && (self.tx.is_some() || self.pulse_start.is_some() || self.pulse_end.is_some()))
    }
    /// Hand the logged transmit changes to `f`, oldest first.
    pub(crate) fn drain_optical(&mut self, mut f: impl FnMut(Time, bool)) {
        let log = &mut self.optical;
        for i in 0..usize::from(log.len) {
            f(log.at[i], log.levels >> i & 1 != 0);
        }
        log.len = 0;
    }
    /// The next basic-clock half with an internal step.
    fn internal_half(&self) -> Option<u64> {
        fn min(a: Option<u64>, b: Option<u64>) -> Option<u64> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, None) => a,
                (None, b) => b,
            }
        }
        // A received pulse ends inside the SCI: while a reception runs, only
        // its samples observe the level (`input_level`); otherwise the end may
        // arm one.
        let mut next = self.receive_pulse.filter(|_| self.rx.is_none());
        if let Some(rx) = self.rx {
            if !rx.format.synchronous {
                next = min(next, rx.next);
            }
        }
        if self.optical.late {
            next = min(
                next,
                if self.lazy(0) {
                    self.tx.and_then(Tx::next)
                } else {
                    self.late_transmit_step()
                },
            );
            next = min(next, self.pulse_start);
            if let Some(PulseEnd::Basic(n)) = self.pulse_end {
                next = min(next, Some(n));
            }
        } else if self.ircr & 0x80 != 0 {
            next = min(next, self.infrared_cell().map(|(next, ..)| next));
        }
        next
    }
    /// The next transmit step that moves at most TXD: a character's start,
    /// or a cell before its stop cell.
    fn late_transmit_step(&self) -> Option<u64> {
        match self.tx? {
            Tx::Start { next, .. } => Some(next),
            Tx::Data {
                character,
                cell,
                next: Some(next),
            } if cell + 1 < character.format.stop() => Some(next),
            _ => None,
        }
    }
    /// In IrDA mode TXD shows only pulses, so a data cell before the stop
    /// cell changes nothing outside the SCI except by the pulse it starts
    /// 13 halves later: the next such cell's half, character and index.
    fn infrared_cell(&self) -> Option<(u64, Character, u8)> {
        if self.ircr & 0x80 == 0 {
            return None;
        }
        match self.tx? {
            Tx::Data {
                character,
                cell,
                next: Some(next),
            } if cell + 1 < character.format.stop() => Some((next, character, cell + 1)),
            _ => None,
        }
    }
    /// The first transmit step that changes something outside the SCI.
    fn transmit_effect(&self) -> Option<u64> {
        if self.optical.late {
            // Every step before the stop cell moves at most TXD.
            return match self.tx? {
                Tx::Start { character, next } => {
                    Some(next + u64::from(character.format.stop()) * character.format.half_bit)
                }
                Tx::Data {
                    character,
                    cell,
                    next: Some(next),
                } if cell + 1 < character.format.stop() => Some(
                    next + u64::from(character.format.stop() - cell - 1)
                        * character.format.half_bit,
                ),
                tx => tx.next(),
            };
        }
        let Some((next, character, first)) = self.infrared_cell() else {
            return self.tx.and_then(Tx::next);
        };
        let stop = character.format.stop();
        (first..=stop).find_map(|cell| {
            let at = next + u64::from(cell - first) * character.format.half_bit;
            if cell == stop {
                Some(at)
            } else {
                (character.word & (1 << cell) == 0).then_some(at + 13)
            }
        })
    }
    /// The half at which the character being received completes, unless it
    /// is abandoned first: the first receive step that changes SSR or RDR.
    fn receive_completion(&self) -> Option<u64> {
        let rx = self.rx?;
        let next = rx.next?;
        if rx.format.synchronous {
            return Some(next);
        }
        let (format, position, next) = if rx.position == -2 {
            let format = self.format();
            (format, -1, next + format.half_bit / 2 - 1)
        } else {
            (rx.format, rx.position, next)
        };
        let stop = i64::from(format.data + u8::from(format.parity));
        let remaining = u64::try_from(stop - i64::from(position)).ok()?;
        Some(next + remaining * format.half_bit)
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
        let optical = core::mem::take(&mut self.optical);
        let light = core::mem::take(&mut self.light);
        *self = Self::default();
        self.optical = optical;
        self.light = light;
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
            transmit: self.transmit_level(),
            receive: self.scr & 0x10 != 0,
        }
    }
    /// Whether every step is internal: late, with no request enabled
    /// (`retained` names enables an instruction still admits) and no clock
    /// output, the steps change only TXD, which is late, and the registers,
    /// which the CPU observes only by accessing them (`flag_deadline`).
    pub(crate) fn lazy(&self, retained: u8) -> bool {
        self.optical.late
            && (self.scr | retained) & 0xc4 == 0
            && self.smr & 0x80 == 0
            && !self.clock_output()
            && self.sync_next.is_none()
    }
    /// While `lazy`: no register changes but by accesses before this time,
    /// the first step that can change SSR or RDR, or a light edge that can
    /// start a reception.
    pub(crate) fn flag_deadline(&self, c: &Clocks) -> Result<Option<Time>, Error> {
        let half = match (self.transmit_effect(), self.receive_completion()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let basic = half
            .map(|n| self.baud.deadline(n, c, self.reload()))
            .transpose()?
            .flatten();
        let light = self
            .light
            .front()
            .filter(|_| self.rx.is_none() && self.scr & 0x10 != 0)
            .map(|(at, _)| at);
        Ok(match (basic, light) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        })
    }
    pub fn deadline(&self, c: &Clocks) -> Result<Option<Time>, Error> {
        self.deadline_with(c, 0)
    }
    pub(crate) fn deadline_with(&self, c: &Clocks, retained: u8) -> Result<Option<Time>, Error> {
        if self.lazy(retained) {
            return Ok(self.mux_glitch);
        }
        let async_clock = (self.clock_output() && self.smr & 0x80 == 0).then(|| {
            let interval = self.format().half_bit / 2;
            self.baud.half + interval - self.baud.half % interval
        });
        let late = self.optical.late;
        let next = [
            self.transmit_effect(),
            self.receive_completion(),
            self.pulse_start.filter(|_| !late),
            self.sync_next,
            async_clock,
            match self.pulse_end {
                Some(PulseEnd::Basic(n)) if !late => Some(n),
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
            Some(PulseEnd::Phi(wait)) if !late => wait.deadline(c)?,
            _ => None,
        };
        // An edge can start a reception, whose completion is an appointment
        // the current state does not show; once one has started, a later
        // start can only complete later.
        let light = self
            .light
            .front()
            .filter(|_| {
                self.rx.is_none()
                    && self.scr & 0x10 != 0
                    && self.ssr & 0x38 == 0
                    && !self.format().synchronous
                    && self.light.route == Some(false)
            })
            .map(|(at, _)| Time::from_raw(at.raw() + 1));
        Ok([basic, pulse, self.mux_glitch, light]
            .into_iter()
            .flatten()
            .min())
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
                    self.baud.reload(self.reload(), now, c);
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
                let (ssr, seen, load) = self.status_write(v);
                (self.ssr, self.seen) = (ssr, seen);
                if load {
                    self.holding = Some(self.tdr);
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
    /// SSR and its read qualification after writing `v` to SSR, and whether
    /// the write loads TDR into the holding register.
    fn status_write(&self, v: u8) -> (u8, u8, bool) {
        let mut ssr = self.ssr & !(self.seen & !v & 0xf8);
        let seen = self.seen & v;
        let mut load = false;
        if self.scr & 0x20 == 0 {
            ssr |= 0x80;
        } else if ssr & 0x80 == 0 && self.holding.is_none() {
            load = true;
            ssr &= !4;
        }
        (ssr, seen, load)
    }
    /// Whether writing `v` to SSR leaves this SCI exactly as it is, whatever
    /// time it is written at. Flags and their qualifications change only at
    /// appointments and by accesses, so this needs no synchronization: such
    /// a write clears nothing and loads nothing, and every step it would
    /// otherwise trigger (a start, an arming, a synchronous clock) has
    /// already been taken where it was due.
    pub(crate) fn inert_status_write(&self, v: u8) -> bool {
        if !self.module || self.reset_mode {
            return true;
        }
        let (ssr, seen, load) = self.status_write(v);
        let starts = match self.tx {
            Some(Tx::Blocked { .. }) => self.ssr & 0x38 == 0,
            Some(Tx::Tail { loaded: None, .. }) => self.holding.is_some(),
            Some(_) => false,
            None => self.scr & 0x20 != 0 && self.holding.is_some(),
        };
        let arms = self.scr & 0x10 != 0
            && self.ssr & 0x38 == 0
            && self.rx.is_none()
            && !self.format().synchronous
            && !self.input_level();
        let clocks = if self.external() {
            self.sync_next.is_some()
        } else if self.sync_needed() {
            self.sync_next.is_none()
        } else {
            self.sync_next.is_some() || !self.sck
        };
        ssr == self.ssr && seen == self.seen && !load && !starts && !arms && !clocks
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
    /// The time of the steps at the current half (`process_basic`).
    fn step_time(&mut self, now: Option<Time>, c: &Clocks) -> Result<Time, Error> {
        match now {
            Some(now) => Ok(now),
            None => self.baud.time(c),
        }
    }
    fn advance_tx(&mut self, now: Option<Time>, c: &Clocks) -> Result<(), Error> {
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
                let now = self.step_time(now, c)?;
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
            Tx::Tail { loaded, .. } => {
                let now = self.step_time(now, c)?;
                self.finish_tail(loaded, now)?;
            }
        }
        Ok(())
    }
    fn input_level(&self) -> bool {
        let physical = self.input ^ (self.spcr & 1 != 0);
        if self.ircr & 0x80 != 0 {
            !(physical || self.receive_pulse.is_some_and(|end| self.baud.half < end))
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
    ) -> Result<bool, Error> {
        if input == self.input && clock.is_none_or(|high| high == self.input_clock) {
            return Ok(false);
        }
        self.sync(now, c)?;
        self.receive_input(input);
        if let Some(high) = clock {
            self.input_clock = high;
            if self.baud.external_edge(now, high)? {
                self.process_basic(Some(now), c)?;
                self.sync_edge(high, now)?;
                self.refresh_sync_clock();
            }
        }
        Ok(true)
    }
    /// Perform the steps at the current half, at `now`, or at the half's
    /// edge when `None`.
    fn process_basic(&mut self, now: Option<Time>, c: &Clocks) -> Result<(), Error> {
        self.expire_receive_pulse();
        if self.pulse_end == Some(PulseEnd::Basic(self.baud.half)) {
            self.infrared = false;
            self.pulse_end = None;
        }
        self.advance_tx(now, c)?;
        if self.pulse_start == Some(self.baud.half) {
            self.pulse_start = None;
            let width = self.ircr >> 4 & 7;
            self.pulse_end = match width {
                0 => Some(PulseEnd::Basic(self.baud.half + 6)),
                n @ 1..=4 => {
                    let now = self.step_time(now, c)?;
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
    fn expire_receive_pulse(&mut self) {
        if self.receive_pulse.is_some_and(|end| end <= self.baud.half) {
            self.receive_pulse = None;
        }
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
        self.process_basic(Some(now), c)?;
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
