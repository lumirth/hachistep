//! IIC2's shared I2C/synchronous shifter, pad filters and open-drain clocks.
//! REJ09B0152-0300 section 16 and applicable A017/A022/A023 corrections.
use super::clocks::{ClockWait, Clocks, Tap};
use crate::{
    error::Error,
    time::{Time, TimeError},
};

const TDRE: u8 = 0x80;
const TEND: u8 = 0x40;
const RDRF: u8 = 0x20;
const NACKF: u8 = 0x10;
const STOP: u8 = 8;
const AL: u8 = 4;
const AAS: u8 = 2;
const ADZ: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HalfWait {
    count: u64,
    paused: bool,
}
impl HalfWait {
    fn after(now: Time, halves: u64, clocks: &Clocks) -> Result<Self, Error> {
        Ok(Self {
            count: clocks
                .system_half_ticks(now)?
                .checked_add(halves)
                .ok_or(TimeError::Overflow)?,
            paused: false,
        })
    }
    fn deadline(self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if self.paused || !clocks.available(Tap::system(1)) {
            return Ok(None);
        }
        Ok(Some(clocks.system_half_edge(self.count)?))
    }
    fn pause(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if !self.paused {
            self.count = self
                .count
                .checked_sub(clocks.system_half_ticks(now)?)
                .ok_or(TimeError::Reversed)?;
            self.paused = true;
        }
        Ok(())
    }
    fn resume(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.paused {
            *self = Self::after(now, self.count, clocks)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Data,
    Ack,
    Done,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Frame {
    shift: u8,
    remaining: u8,
    phase: Phase,
    address: bool,
    transmit: bool,
    loaded: bool,
    last: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Condition {
    StartRelease,
    StartHigh,
    StartHold,
    StopLow,
    StopHigh,
    StopRelease,
    StopDetect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hold {
    Transmit,
    Receive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iic {
    control: u8,
    mode: u8,
    count: u8,
    enable: u8,
    status: u8,
    seen: u8,
    address: u8,
    tdr: u8,
    rdr: u8,
    busy: bool,
    reset: bool,
    gate: bool,
    /// true releases the open-drain output; this is not the pad level.
    drive: [bool; 2],
    raw: [bool; 2],
    latch: [bool; 2],
    filtered: [bool; 2],
    filter_wait: Option<ClockWait>,
    clock_wait: Option<HalfWait>,
    monitor_wait: Option<HalfWait>,
    setup_wait: Option<ClockWait>,
    synchronized_hold: bool,
    condition: Option<Condition>,
    command: Option<bool>,
    frame: Option<Frame>,
    selected: bool,
    completed: bool,
    hold: Option<Hold>,
    receive_enabled: bool,
    receive_stopped: bool,
    eighth_fall: Option<Time>,
    stale_release: bool,
}
impl Default for Iic {
    fn default() -> Self {
        Self {
            control: 0,
            mode: 0,
            count: 0,
            enable: 0,
            status: 0,
            seen: 0,
            address: 0,
            tdr: 0xff,
            rdr: 0xff,
            busy: false,
            reset: false,
            gate: false,
            drive: [true; 2],
            raw: [true; 2],
            latch: [true; 2],
            filtered: [true; 2],
            filter_wait: None,
            clock_wait: None,
            monitor_wait: None,
            setup_wait: None,
            synchronized_hold: false,
            condition: None,
            command: None,
            frame: None,
            selected: false,
            completed: false,
            hold: None,
            receive_enabled: false,
            receive_stopped: false,
            eighth_fall: None,
            stale_release: false,
        }
    }
}
impl Iic {
    fn enabled(&self) -> bool {
        self.control & 0x80 != 0
    }
    fn master(&self) -> bool {
        self.control & 0x20 != 0
    }
    fn transmit(&self) -> bool {
        self.control & 0x10 != 0
    }
    fn synchronous(&self) -> bool {
        self.address & 1 != 0
    }
    fn period(&self) -> u64 {
        [28, 40, 48, 64, 80, 100, 112, 128][usize::from(self.control & 7)]
            << ((self.control >> 3) & 1)
    }
    pub fn pins(&self) -> Option<[bool; 2]> {
        self.enabled().then_some(self.drive)
    }
    pub fn interrupt(&self) -> bool {
        self.interrupt_with_enable(0)
    }
    pub(crate) fn interrupt_with_enable(&self, retained: u8) -> bool {
        let enable = self.enable | retained;
        self.status & enable & 0xe0 != 0
            || (!self.synchronous() && self.status & enable & STOP != 0)
            || (enable & NACKF != 0
                && self.status & (AL | if self.synchronous() { 0 } else { NACKF }) != 0)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address & 7 {
            0 => self.control,
            1 => {
                u8::from(self.busy) << 7
                    | 0x55
                    | u8::from(self.drive[1]) << 5
                    | u8::from(self.drive[0]) << 3
                    | u8::from(self.reset) << 1
            }
            2 => self.mode | 0x38 | self.frame.map_or(self.count, |f| f.remaining & 7),
            3 => self.enable,
            4 => self.status,
            5 => self.address,
            6 => self.tdr,
            _ => self.rdr,
        }
    }
    pub fn read(&mut self, address: u16, now: Time, clocks: &Clocks) -> Result<u8, Error> {
        let value = self.peek(address);
        match address & 7 {
            4 => self.seen = self.status,
            7 => {
                self.status &= !RDRF;
                self.seen &= !RDRF;
                if self.eighth_fall == Some(now) {
                    self.stale_release = true;
                }
                if !self.transmit() {
                    if self.master() && !self.receive_stopped {
                        self.receive_enabled = true;
                        if let Some(f) = self.frame.as_mut() {
                            if f.phase != Phase::Done && !f.address {
                                f.last = self.control & 0x40 != 0;
                            }
                        }
                    }
                    if self.hold.is_some() {
                        self.release_hold(now, clocks)?;
                    }
                    self.kick(now, clocks)?;
                }
            }
            _ => {}
        }
        Ok(value)
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        match address & 7 {
            0 => {
                let previous = self.control;
                self.control = value;
                if previous & 0x10 == 0 && self.transmit() {
                    self.status |= TDRE;
                }
                if previous & 0x30 != value & 0x30 {
                    self.receive_enabled = self.synchronous() && !self.transmit();
                    self.receive_stopped = false;
                    let transmit = self.transmit();
                    let master = self.master();
                    if let Some(f) = self.frame.as_mut() {
                        if f.phase == Phase::Data && !self.filtered[0] {
                            f.transmit = transmit && (master || !f.address);
                            f.loaded |= !f.transmit;
                            f.last = !transmit && value & 0x40 != 0;
                        }
                    }
                }
                if !self.enabled() {
                    self.stop_shifter();
                    self.filter_wait = None;
                } else {
                    self.schedule_filter(now, clocks)?;
                    if self.synchronous()
                        && !self.transmit()
                        && self.control & 0x40 != 0
                        && self.frame.is_none()
                    {
                        self.receive_enabled = false;
                    }
                    self.kick(now, clocks)?;
                }
            }
            1 => {
                let held = self.reset;
                self.reset = value & 2 != 0;
                if self.reset {
                    self.stop_shifter();
                    if self.transmit() {
                        self.status |= TDRE;
                    }
                }
                if !held && !self.reset {
                    if value & 0x10 == 0 {
                        self.drive[1] = value & 0x20 != 0;
                    }
                    if value & 0x40 == 0 && self.master() && !self.synchronous() && self.enabled() {
                        let start = value & 0x80 != 0;
                        // A023: ACKE can lose STOP before the ninth falling edge.
                        let early_stop = !start
                            && self.enable & 4 != 0
                            && self.transmit()
                            && self.filtered[0]
                            && self.frame.is_some_and(|f| f.phase == Phase::Done);
                        if !early_stop {
                            self.command = Some(start);
                            self.kick(now, clocks)?;
                        }
                    }
                }
            }
            2 => {
                self.mode = value & 0xc0;
                if value & 8 == 0 {
                    self.count = value & 7;
                    if let Some(f) = self.frame.as_mut() {
                        if f.phase == Phase::Data && !f.address {
                            f.remaining = if self.count == 0 { 8 } else { self.count };
                        }
                    }
                }
            }
            3 => self.enable = (value & !2) | (self.enable & 2),
            4 => {
                self.status &= !(self.seen & !value);
                self.seen &= self.status;
                self.kick(now, clocks)?;
            }
            5 => {
                self.address = value;
                if self.synchronous() {
                    self.busy = false;
                }
                self.kick(now, clocks)?;
            }
            6 => {
                self.tdr = if self.mode & 0x80 == 0 {
                    value
                } else {
                    value.reverse_bits()
                };
                self.status &= !(TDRE | TEND);
                self.seen &= !(TDRE | TEND);
                if self.hold == Some(Hold::Transmit) {
                    self.load_transmit();
                    self.release_hold(now, clocks)?;
                }
                self.kick(now, clocks)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn stop_shifter(&mut self) {
        self.drive = [true; 2];
        self.frame = None;
        self.clock_wait = None;
        self.monitor_wait = None;
        self.setup_wait = None;
        self.condition = None;
        self.command = None;
        self.hold = None;
        self.synchronized_hold = false;
        self.receive_enabled = false;
        self.receive_stopped = false;
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.gate == gate {
            return Ok(());
        }
        self.gate = gate;
        for wait in [&mut self.clock_wait, &mut self.monitor_wait]
            .into_iter()
            .flatten()
        {
            if gate {
                wait.resume(now, clocks)?;
            } else {
                wait.pause(now, clocks)?;
            }
        }
        if gate && self.synchronized_hold {
            if let Some(w) = &mut self.clock_wait {
                w.pause(now, clocks)?;
            }
        }
        for wait in [&mut self.filter_wait, &mut self.setup_wait]
            .into_iter()
            .flatten()
        {
            if gate {
                wait.resume(now, clocks)?;
            } else {
                wait.pause(now, clocks)?;
            }
        }
        if gate {
            self.schedule_filter(now, clocks)?;
            self.kick(now, clocks)?;
        }
        Ok(())
    }
    fn schedule_filter(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.gate
            && self.enabled()
            && self.filter_wait.is_none()
            && (self.raw != self.latch || self.latch != self.filtered)
        {
            self.filter_wait = Some(ClockWait::after(now, 1, Tap::system(1), clocks)?);
        }
        Ok(())
    }
    pub fn input_pins(
        &mut self,
        scl: bool,
        sda: bool,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        self.raw = [scl, sda];
        self.schedule_filter(now, clocks)
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        if !self.gate || !self.enabled() {
            return Ok(None);
        }
        Ok([
            self.filter_wait
                .as_ref()
                .map_or(Ok(None), |w| w.deadline(clocks))?,
            self.clock_wait.map_or(Ok(None), |w| w.deadline(clocks))?,
            self.monitor_wait.map_or(Ok(None), |w| w.deadline(clocks))?,
            self.setup_wait
                .as_ref()
                .map_or(Ok(None), |w| w.deadline(clocks))?,
        ]
        .into_iter()
        .flatten()
        .min())
    }
    pub fn advance(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if !self.gate || !self.enabled() {
            return Ok(());
        }
        if self
            .filter_wait
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(clocks))?
            == Some(now)
        {
            self.filter_wait = None;
            let old = self.filtered;
            for (i, pad) in self.raw.into_iter().enumerate() {
                if pad == self.latch[i] {
                    self.filtered[i] = pad;
                }
                self.latch[i] = pad;
            }
            self.filtered_edges(old, now, clocks)?;
            self.schedule_filter(now, clocks)?;
        }
        if self
            .setup_wait
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(clocks))?
            == Some(now)
        {
            self.setup_wait = None;
            self.drive[0] = true;
        }
        if self.monitor_wait.map_or(Ok(None), |w| w.deadline(clocks))? == Some(now) {
            self.monitor_wait = None;
            if !self.filtered[0] {
                if let Some(w) = &mut self.clock_wait {
                    w.pause(now, clocks)?;
                }
                self.synchronized_hold = true;
            }
        }
        if self.clock_wait.map_or(Ok(None), |w| w.deadline(clocks))? == Some(now) {
            self.clock_wait = None;
            self.clock_step(now, clocks)?;
        }
        self.kick(now, clocks)
    }
    fn filtered_edges(&mut self, old: [bool; 2], now: Time, clocks: &Clocks) -> Result<(), Error> {
        let [scl, sda] = self.filtered;
        if !self.synchronous() && old[0] && scl && old[1] != sda {
            if sda {
                self.busy = false;
                self.condition = None;
                self.clock_wait = None;
                self.monitor_wait = None;
                if self.selected || (self.master() && self.completed) {
                    self.status |= STOP;
                }
                self.frame = None;
                self.selected = false;
                self.receive_enabled = false;
                self.receive_stopped = false;
                self.hold = None;
                if !self.reset {
                    self.drive[1] = true;
                }
            } else {
                if self.master() && self.drive[1] {
                    self.arbitration_lost();
                }
                self.busy = true;
                self.selected = false;
                self.completed = false;
                self.receive_stopped = false;
                if !self.reset {
                    self.begin_frame(true);
                }
            }
        }
        if self.reset {
            return Ok(());
        }
        if self.synchronized_hold && scl {
            self.synchronized_hold = false;
            if let Some(w) = &mut self.clock_wait {
                w.resume(now, clocks)?;
            }
        }
        if self.condition == Some(Condition::StartHigh) && scl && sda {
            self.drive[1] = false;
            self.condition = Some(Condition::StartHold);
            self.clock_wait = Some(HalfWait::after(now, self.period(), clocks)?);
        } else if self.condition == Some(Condition::StopHigh) && scl {
            self.condition = Some(Condition::StopRelease);
            self.clock_wait = Some(HalfWait::after(now, self.period(), clocks)?);
        }
        if old[0] != scl {
            if scl {
                self.rising(sda);
            } else {
                self.falling(now, clocks)?;
            }
        }
        Ok(())
    }
    fn begin_frame(&mut self, address: bool) {
        let transmit =
            self.transmit() && (self.master() || (self.selected && !address) || self.synchronous());
        self.frame = Some(Frame {
            shift: 0,
            remaining: if address || self.count == 0 {
                8
            } else {
                self.count
            },
            phase: Phase::Data,
            address,
            transmit,
            loaded: !transmit,
            last: !transmit && self.master() && self.control & 0x40 != 0,
        });
        self.load_transmit();
    }
    fn load_transmit(&mut self) {
        if let Some(f) = self.frame.as_mut() {
            if f.transmit && !f.loaded && self.status & TDRE == 0 {
                f.shift = self.tdr;
                f.loaded = true;
                self.status |= TDRE;
            }
        }
    }
    fn arbitration_lost(&mut self) {
        self.status |= AL;
        self.control &= !0x30;
        self.drive = [true; 2];
        self.clock_wait = None;
        self.monitor_wait = None;
        self.synchronized_hold = false;
        self.condition = None;
        self.command = None;
        self.hold = None;
        if let Some(f) = self.frame.as_mut() {
            f.transmit = false;
            f.loaded = true;
        }
    }
    fn rising(&mut self, sda: bool) {
        let Some(mut f) = self.frame else {
            return;
        };
        if !f.loaded {
            return;
        }
        match f.phase {
            Phase::Data => {
                if f.transmit && self.master() && self.drive[1] && !sda && !self.synchronous() {
                    self.arbitration_lost();
                    f.transmit = false;
                }
                f.shift = (f.shift << 1) | u8::from(sda);
                f.remaining -= 1;
                if f.remaining == 0 {
                    if f.address && !self.master() {
                        self.selected = f.shift & 0xfe == self.address & 0xfe || f.shift == 0;
                        if self.selected {
                            self.status |= AAS;
                            if f.shift == 0 {
                                self.status |= ADZ;
                            }
                            if f.shift & 1 != 0 {
                                self.control |= 0x10;
                                self.status |= TDRE;
                            } else {
                                self.control &= !0x10;
                            }
                        }
                    }
                    if self.synchronous() {
                        self.complete_frame(&f, sda);
                        f.phase = Phase::Done;
                        if f.last {
                            self.receive_stopped = true;
                            self.receive_enabled = false;
                            self.clock_wait = None;
                            self.monitor_wait = None;
                            self.drive[0] = true;
                        }
                    } else {
                        f.phase = Phase::Ack;
                    }
                }
            }
            Phase::Ack => {
                self.complete_frame(&f, sda);
                f.phase = Phase::Done;
            }
            Phase::Done => {}
        }
        self.frame = Some(f);
    }
    fn complete_frame(&mut self, f: &Frame, sda: bool) {
        self.count = 0;
        self.completed = true;
        if f.transmit {
            if !self.synchronous() {
                self.enable = self.enable & !2 | u8::from(sda) << 1;
                if sda && self.enable & 4 != 0 {
                    self.status |= NACKF;
                }
            }
            if self.status & TDRE != 0 {
                self.status |= TEND;
            }
        } else if (self.master() || self.selected || self.synchronous())
            && !(f.address && f.shift & 1 != 0)
        {
            if self.synchronous() && self.status & RDRF != 0 {
                self.status |= AL;
                self.control &= !0x20;
                self.clock_wait = None;
                self.monitor_wait = None;
                self.drive[0] = true;
            } else {
                self.rdr = if self.mode & 0x80 != 0 {
                    f.shift.reverse_bits()
                } else {
                    f.shift
                };
                self.status |= RDRF;
            }
        }
    }
    fn falling(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.condition.is_some() {
            return Ok(());
        }
        if self.command.is_some() {
            return self.kick(now, clocks);
        }
        if self.frame.is_some_and(|f| f.phase == Phase::Done) {
            let f = self.frame.take().unwrap();
            if f.last {
                self.receive_enabled = false;
                self.receive_stopped = true;
            }
            self.drive[1] = true;
        }
        self.kick(now, clocks)?;
        let transmit = self.transmit();
        let master = self.master();
        if let Some(f) = self.frame.as_mut() {
            if f.phase == Phase::Data {
                f.transmit = transmit && (master || !f.address);
                f.loaded |= !f.transmit;
            }
        }
        let Some(f) = self.frame else {
            if self.master() {
                self.clock_wait = None;
                self.monitor_wait = None;
                self.drive[0] = false;
            }
            return Ok(());
        };
        match f.phase {
            Phase::Data => {
                self.load_transmit();
                let f = self.frame.unwrap();
                if f.transmit && !f.loaded {
                    self.set_hold(Hold::Transmit);
                    if self.synchronous() && self.master() {
                        self.clock_wait = None;
                        self.monitor_wait = None;
                    }
                } else {
                    self.drive[1] = !f.transmit || f.shift & 0x80 != 0;
                }
            }
            Phase::Ack => {
                self.eighth_fall = Some(now);
                if !f.transmit
                    && (self.selected || self.master())
                    && self.status & RDRF != 0
                    && !self.stale_release
                {
                    self.set_hold(Hold::Receive);
                }
                self.stale_release = false;
                self.drive[1] =
                    f.transmit || (!self.selected && !self.master()) || self.enable & 1 != 0;
                if self.master() && self.mode & 0x40 != 0 && self.hold.is_none() {
                    let extra = self.period() * 4;
                    if let Some(wait) = &mut self.clock_wait {
                        wait.count = wait.count.checked_add(extra).ok_or(TimeError::Overflow)?;
                    }
                }
            }
            Phase::Done => {}
        }
        Ok(())
    }
    fn set_hold(&mut self, hold: Hold) {
        if self.synchronous() {
            return;
        }
        self.hold = Some(hold);
        self.drive[0] = false;
        self.clock_wait = None;
        self.monitor_wait = None;
    }
    fn release_hold(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        self.hold = None;
        if self.master() {
            self.clock_wait = Some(HalfWait::after(now, self.period(), clocks)?);
        } else {
            let cycles = if self.control & 8 == 0 { 10 } else { 20 };
            self.setup_wait = Some(ClockWait::after(now, cycles, Tap::system(1), clocks)?);
        }
        if let Some(f) = self.frame {
            self.drive[1] = if f.phase == Phase::Ack {
                self.enable & 1 != 0
            } else {
                !f.transmit || f.shift & 0x80 != 0
            };
        }
        Ok(())
    }
    fn kick(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if !self.enabled() || !self.gate || self.reset {
            return Ok(());
        }
        if self.condition.is_some() {
            return Ok(());
        }
        if self.command.is_some() && (self.frame.is_none() || !self.filtered[0]) {
            let start = self.command.take().unwrap();
            self.frame = None;
            self.hold = None;
            self.monitor_wait = None;
            self.synchronized_hold = false;
            self.drive = [false, start];
            self.condition = Some(if start {
                Condition::StartRelease
            } else {
                Condition::StopLow
            });
            self.clock_wait = Some(HalfWait::after(now, self.period(), clocks)?);
            return Ok(());
        }
        if self.frame.is_none() && (self.busy || self.synchronous()) {
            let can_transmit = self.transmit()
                && (self.master() || self.selected || self.synchronous())
                && self.status & NACKF == 0;
            let can_receive = !self.transmit()
                && (if self.master() {
                    self.receive_enabled
                } else {
                    self.selected || self.synchronous()
                });
            if can_transmit || can_receive {
                self.begin_frame(false);
            }
        }
        self.load_transmit();
        if self.master()
            && self.frame.is_some()
            && self.clock_wait.is_none()
            && self.hold.is_none()
            && !self.synchronized_hold
        {
            let f = self.frame.unwrap();
            if f.phase == Phase::Done {
                return Ok(());
            }
            if f.transmit && !f.loaded {
                self.set_hold(Hold::Transmit);
                return Ok(());
            }
            if self.drive[0] {
                self.drive[0] = false;
            }
            if !self.filtered[0] {
                self.drive[1] = if f.phase == Phase::Data {
                    !f.transmit || f.shift & 0x80 != 0
                } else {
                    f.transmit || self.enable & 1 != 0
                };
            }
            self.clock_wait = Some(HalfWait::after(now, self.period(), clocks)?);
        }
        Ok(())
    }
    fn clock_step(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        match self.condition {
            Some(Condition::StartRelease) => {
                self.drive[0] = true;
                self.condition = Some(Condition::StartHigh);
            }
            Some(Condition::StartHold) => {
                self.condition = None;
                self.drive[0] = false;
                // The filtered falling edge launches the first address bit.
                self.clock_wait = Some(HalfWait::after(now, self.period(), clocks)?);
            }
            Some(Condition::StopLow) => {
                self.drive[0] = true;
                self.condition = Some(Condition::StopHigh);
            }
            Some(Condition::StopRelease) => {
                self.drive[1] = true;
                self.condition = Some(Condition::StopDetect);
                self.frame = None;
                self.receive_enabled = false;
            }
            Some(_) => {}
            None if self.master() && self.hold.is_none() => {
                self.drive[0] = !self.drive[0];
                self.clock_wait = Some(HalfWait::after(now, self.period(), clocks)?);
                if self.drive[0] {
                    let halves = [15, 39, 35, 83][usize::from((self.control >> 2) & 3)];
                    self.monitor_wait = Some(HalfWait::after(now, halves, clocks)?);
                } else {
                    self.monitor_wait = None;
                }
            }
            None => {}
        }
        Ok(())
    }
}
