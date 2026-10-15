//! SSU holding registers and a shifter that advances on each clock edge.
//! The board routes each emitted edge to the attached devices.
use super::clocks::{ClockWait, Clocks, Tap};
use crate::{error::Error, signals::Drive, time::Time};
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Phase {
    Load = 0,
    Edge = 1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub clock: bool,
    pub mosi: bool,
    pub sample: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pins {
    /// Logical SCS, SSCK, SSO, SSI; None retains the GPIO function.
    pub drives: [Option<Drive>; 4],
    /// SOOS also applies to GPIO use of SSO/SSI, independently of TE/RE.
    pub data_open_drain: bool,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Ssu {
    high: u8,
    low: u8,
    mode: u8,
    enable: u8,
    status: u8,
    seen: u8,
    tdr: u8,
    rdr: u8,
    receive_started: bool,
    active: bool,
    transmitting: bool,
    select_active: bool,
    select_high: bool,
    external_clock: bool,
    sol_protected: bool,
    msb_first: bool,
    tx: u8,
    rx: u8,
    edges: u8,
    shifted: u8,
    sampled: u8,
    clock_high: bool,
    mosi: bool,
    next: Option<ClockWait>,
    phase: Phase,
    gate: bool,
    #[borsh(skip)]
    pub transmitted: u64,
    #[borsh(skip)]
    pub received: u64,
}
impl Default for Ssu {
    fn default() -> Self {
        Self {
            high: 8,
            low: 0,
            mode: 0,
            enable: 0,
            status: 4,
            seen: 0,
            tdr: 0,
            rdr: 0,
            receive_started: false,
            active: false,
            transmitting: false,
            select_active: false,
            select_high: true,
            external_clock: true,
            sol_protected: true,
            msb_first: false,
            tx: 0,
            rx: 0,
            edges: 0,
            shifted: 0,
            sampled: 0,
            clock_high: true,
            mosi: false,
            next: None,
            phase: Phase::Load,
            gate: false,
            transmitted: 0,
            received: 0,
        }
    }
}
impl Ssu {
    /// SSCRH includes live MOSI. RDR consumption can start reception. Other
    /// reads expose retained registers or flags changed at effect appointments.
    pub(crate) const fn reads_shift(address: u16) -> bool {
        matches!(address, 0xf0e0 | 0xf0e9)
    }
    /// Only receive-only RDR consumption can schedule a frame. Duplex reads
    /// consume RDRF but leave the shifter's clock obligations unchanged.
    pub(crate) fn read_rearms(&self, address: u16) -> bool {
        address == 0xf0e9 && self.enable & 0xc0 == 0x40
    }
    /// Retained registers, and status flags that change only at the effect
    /// deadline; repeating a status read qualifies the same flags again.
    pub(crate) const fn settled_read(address: u16) -> bool {
        matches!(address, 0xf0e1..=0xf0e4 | 0xf0eb)
    }
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        self.next
            .as_ref()
            .map_or(Ok(None), |wait| wait.deadline(clocks))
    }
    /// The last receive sample and frame completion can change CPU-visible flags.
    pub(crate) fn effect_deadline(
        &self,
        edges: u8,
        clocks: &Clocks,
    ) -> Result<Option<Time>, Error> {
        let count = match self.phase {
            Phase::Load => 1,
            Phase::Edge => {
                let sample = if self.first_edge_samples() { 15u8 } else { 16 };
                edges.min(sample.saturating_sub(self.edges).max(1))
            }
        };
        self.next.as_ref().map_or(Ok(None), |wait| {
            wait.later_edge(u64::from(count - 1), clocks)
        })
    }
    /// Upcoming shift clock edges strictly before `end`.
    pub(crate) fn edges_before(&self, end: Time, clocks: &Clocks) -> u64 {
        self.next
            .as_ref()
            .map_or(0, |wait| wait.edges_before(end, clocks))
    }
    pub(crate) fn later_edge(&self, count: u64, clocks: &Clocks) -> Result<Option<Time>, Error> {
        self.next
            .as_ref()
            .map_or(Ok(None), |wait| wait.later_edge(count, clocks))
    }
    /// Whether `input_pins` would leave the observed select level unchanged.
    pub(crate) fn sees_select(&self, high: bool) -> bool {
        self.select_high == high
    }
    /// One of `quiet_edges`, whose clock obligation `follow_quiet` settles.
    pub(crate) fn quiet_edge(&mut self) -> Edge {
        self.clock_high = !self.clock_high;
        self.shift_edge()
    }
    pub(crate) fn follow_quiet(&mut self, edges: u64, clocks: &Clocks) -> Result<(), Error> {
        self.next
            .as_mut()
            .ok_or(Error::Internal("SSU edge without clock obligation"))?
            .follow_by(edges, clocks)
    }
    pub(crate) fn master(&self) -> bool {
        self.high & 0x80 != 0
    }
    fn four_line(&self) -> bool {
        self.low & 0x40 != 0
    }
    fn bidirectional(&self) -> bool {
        self.four_line() && self.high & 0x40 != 0
    }
    fn selected(&self) -> bool {
        !self.four_line() || self.high & 3 == 0 || !self.select_high
    }
    pub fn input_pin(&self) -> usize {
        if self.four_line() && (!self.master() || self.bidirectional()) {
            2 // SSO
        } else {
            3 // SSI
        }
    }
    /// Logical SCS, SSCK, SSO, SSI functions, before the package's SSEL mux.
    /// None leaves the pin to GPIO; Floating selects a released serial pin.
    pub fn pins(&self) -> Pins {
        let mut pins = Pins {
            data_open_drain: self.high & 0x20 != 0,
            ..Pins::default()
        };
        if !self.gate {
            return pins;
        }
        if self.four_line() && self.high & 3 != 0 {
            pins.drives[0] = Some(
                if self.master() && self.high & 2 != 0 && self.select_active {
                    Drive::Low
                } else {
                    Drive::Floating
                },
            );
        }
        if self.enable & 0x40 != 0 {
            pins.drives[self.input_pin()] = Some(Drive::Floating);
        }
        self.shift_pins(pins)
    }
    /// `pins` with the drivers that follow the shift clock and data output
    /// replaced by their current levels: the only ones an edge changes.
    #[inline]
    pub(crate) fn shift_pins(&self, pins: Pins) -> Pins {
        self.shift_pins_at(pins, self.clock_high, self.mosi)
    }
    /// The shift clock and data output levels: bit 0 and bit 1.
    pub(crate) fn shift_levels(&self) -> usize {
        usize::from(self.clock_high) | usize::from(self.mosi) << 1
    }
    /// Whether the due appointment is a shift clock edge of a master frame.
    pub(crate) fn shifting(&self) -> bool {
        self.phase == Phase::Edge && self.active && self.master() && self.next.is_some()
    }
    /// `shift_pins` with the shift clock and data output at the given levels.
    pub(crate) fn shift_pins_at(&self, mut pins: Pins, clock_high: bool, mosi: bool) -> Pins {
        if !self.gate {
            return pins;
        }
        let drive = |high: bool, open_drain: bool| {
            if !high {
                Drive::Low
            } else if open_drain {
                Drive::Floating
            } else {
                Drive::High
            }
        };
        if self.high & 4 != 0 {
            pins.drives[1] = Some(if self.master() {
                drive(clock_high, self.low & 0x10 != 0)
            } else {
                Drive::Floating
            });
        }
        if self.enable & 0x80 != 0 {
            let output = if self.four_line() && !self.master() && !self.bidirectional() {
                3
            } else {
                2
            };
            let enabled = if self.master() && self.four_line() && self.high & 2 != 0 {
                self.select_active
            } else {
                self.selected()
            };
            pins.drives[output] = Some(if enabled {
                drive(mosi, self.high & 0x20 != 0)
            } else {
                Drive::Floating
            });
        }
        pins
    }
    /// Everything `pins` depends on. Equal states have equal pins.
    pub(crate) fn pin_state(&self) -> [u8; 5] {
        [
            self.high,
            self.low,
            self.enable,
            u8::from(self.gate)
                | u8::from(self.select_active) << 1
                | u8::from(self.select_high) << 2
                | u8::from(self.clock_high) << 3
                | u8::from(self.mosi) << 4,
            0,
        ]
    }
    /// `pin_state` without what a shift net indexes or ignores: the shift
    /// outputs, and the master's select drive without an SCS function.
    pub(crate) fn pin_key(&self) -> [u8; 5] {
        let mut key = self.pin_state();
        key[3] &= !0x18;
        if !self.select_pin() {
            key[3] &= !0x02;
        }
        key
    }
    /// Whether frame loads and completions never move a pin: no select
    /// output, the clock already at its idle level, and no data output
    /// change while an idle-high clock is high.
    pub(crate) fn plain_frames(&self) -> bool {
        !self.select_pin() && !(self.first_edge_samples() && self.idle_high())
    }
    /// Whether SCS is the SSU's: the select input and output of four-line
    /// mode. Otherwise neither the select level nor `select_active` reaches
    /// the shifter or the pins.
    fn select_pin(&self) -> bool {
        self.four_line() && self.high & 3 != 0
    }
    /// Whether the due load moves no pin but the data output, with the
    /// shift clock low if it does (`plain_frames`).
    pub(crate) fn loads_plainly(&self) -> bool {
        self.phase == Phase::Load && self.plain_frames() && self.clock_high == self.idle_high()
    }
    /// Whether any status flag can request an interrupt.
    pub(crate) fn interrupts_enabled(&self) -> bool {
        self.enable & 0x0f != 0
    }
    /// No status flag changes before this time but by register accesses:
    /// flags change at loads, at the sample completing reception and at frame
    /// completion, where a completion that sets none can still schedule a
    /// load one CPU state later.
    pub(crate) fn flag_deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        let Some(next) = &self.next else {
            return Ok(None);
        };
        if self.phase == Phase::Load {
            return next.deadline(clocks);
        }
        let last = u64::from(15 - self.edges.min(15));
        let finish = next
            .later_edge(last, clocks)?
            .ok_or(Error::Internal("SSU edge without clock obligation"))?;
        let completes = if self.transmitting && self.enable & 0x80 != 0 && self.status & 4 != 0 {
            finish
        } else {
            clocks.after(finish, 1, Tap::cpu())?
        };
        if self.enable & 0x40 != 0 && self.sampled < 8 {
            let sample = if self.first_edge_samples() { 15u8 } else { 16 };
            if let Some(index) = sample.checked_sub(self.edges + 1) {
                if let Some(at) = next.later_edge(u64::from(index), clocks)? {
                    return Ok(Some(at.min(completes)));
                }
            }
        }
        Ok(Some(completes))
    }
    /// `deadline` and `flag_deadline` as CPU clock ordinals of the edges
    /// they fall on, when both are expressible so (`ClockWait::cpu_edge`).
    pub(crate) fn due_ordinals(&self, clocks: &Clocks) -> Option<(Option<u64>, Option<u64>)> {
        let Some(next) = &self.next else {
            return Some((None, None));
        };
        let (deadline, period) = next.cpu_run(clocks)?;
        let edge = |count: u8| deadline.checked_add(u64::from(count).checked_mul(period)?);
        if self.phase == Phase::Load {
            return Some((Some(deadline), Some(deadline)));
        }
        let finish = edge(15 - self.edges.min(15))?;
        // The next CPU edge after one on the CPU clock is the one after it.
        let completes = if self.transmitting && self.enable & 0x80 != 0 && self.status & 4 != 0 {
            finish
        } else {
            finish.checked_add(1)?
        };
        if self.enable & 0x40 != 0 && self.sampled < 8 {
            let sample = if self.first_edge_samples() { 15u8 } else { 16 };
            if let Some(index) = sample.checked_sub(self.edges + 1) {
                let at = edge(index)?;
                return Some((Some(deadline), Some(at.min(completes))));
            }
        }
        Some((Some(deadline), Some(completes)))
    }
    /// The CPU clock ordinal of the `count`-th upcoming edge, when
    /// expressible so (`ClockWait::cpu_edge`).
    pub(crate) fn cpu_edge(&self, count: u64, clocks: &Clocks) -> Option<u64> {
        self.next.as_ref()?.cpu_edge(count, clocks)
    }
    pub(crate) fn cpu_run(&self, clocks: &Clocks) -> Option<(u64, u64)> {
        self.next.as_ref()?.cpu_run(clocks)
    }
    /// Edges left in the running frame.
    pub(crate) fn frame_edges(&self) -> u64 {
        u64::from(16 - self.edges)
    }
    /// MSB-first falling/rising pairs, starting between samples. A partial
    /// leading or trailing edge remains with the ordinary edge shifter.
    pub(crate) fn output_bits(&self, count: u8) -> Option<u8> {
        if !self.shifting() || !self.clock_high || self.first_edge_samples() || !self.msb_first {
            return None;
        }
        let mask = 0xff >> (8 - count);
        Some(if self.transmitting {
            (self.tx >> (8 - self.shifted - count)) & mask
        } else if self.mosi {
            mask
        } else {
            0
        })
    }
    pub(crate) fn shifted_bits(&mut self, input: u8, count: u8) {
        self.edges += 2 * count;
        if self.transmitting {
            self.shifted += count;
            self.mosi = self.output_bit(self.shifted - 1);
        }
        self.rx = ((u16::from(self.rx) << count) | u16::from(input)) as u8;
        self.sampled += count;
        if self.sampled == 8 {
            self.finish_sample();
        }
    }
    /// Settle the clock obligation after `edges` of `quiet_edge`: the frame
    /// ends at its sixteenth edge.
    pub(crate) fn shifted(&mut self, edges: u64, clocks: &Clocks) -> Result<(), Error> {
        if self.edges == 16 {
            self.next = None;
            Ok(())
        } else {
            self.follow_quiet(edges, clocks)
        }
    }
    /// Whether a TDR write now leaves the load schedule alone: a frame is
    /// shifting, or a load is already due, so the new byte waits for it
    /// (`schedule_load`).
    pub(crate) fn loads_later_on_transmit(&self) -> bool {
        if self.active {
            // A receive-enabled slave still waiting for its first edge
            // abandons that frame for the transmit byte.
            self.master() || self.edges != 0 || self.transmitting
        } else {
            self.next.is_some()
        }
    }
    /// Whether `other` keeps this owner's frame and its clock obligation.
    pub(crate) fn same_schedule(&self, other: &Self) -> bool {
        self.next == other.next && self.phase == other.phase && self.active == other.active
    }
    /// Whether `finish_edge` completes the frame now.
    pub(crate) fn finishing(&self) -> bool {
        self.active && self.edges == 16 && self.next.is_none()
    }
    pub fn uses_subclock(&self) -> bool {
        self.mode & 7 == 7
    }
    fn half_period(&self) -> Tap {
        match self.mode & 7 {
            0 => Tap::system(128),
            1 => Tap::system(64),
            2 => Tap::system(32),
            3 => Tap::system(16),
            4 => Tap::system(8),
            5 => Tap::system(4),
            6 => Tap::system(2),
            _ => Tap::subclock(),
        }
    }
    fn idle_high(&self) -> bool {
        self.mode & 0x40 == 0
    }
    fn first_edge_samples(&self) -> bool {
        self.mode & 0x20 != 0
    }
    fn output_bit(&self, index: u8) -> bool {
        let shift = if self.msb_first { 7 - index } else { index };
        self.tx & (1u8 << shift) != 0
    }
    fn schedule_load(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        self.schedule_load_at(Instant::time(now), clocks)
    }
    fn schedule_load_at(&mut self, now: Instant, clocks: &Clocks) -> Result<(), Error> {
        self.schedule_load_with(|| Ok(now), clocks)
    }
    fn schedule_load_with(
        &mut self,
        now: impl FnOnce() -> Result<Instant, Error>,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        let overrun = self.status & 0x40 != 0;
        let transmit =
            self.enable & 0x80 != 0 && self.status & 4 == 0 && (!self.master() || !overrun);
        let receive = self.enable & 0x40 != 0
            && !overrun
            && (!self.master() || (self.enable & 0x80 == 0 && self.receive_started));
        // A receive-enabled slave can be waiting for its very first external
        // edge when software supplies transmit data. The empty shifter can
        // still accept that byte; preparing reception did not consume a frame.
        if !self.master() && self.active && self.edges == 0 && !self.transmitting && transmit {
            self.active = false;
        }
        if !self.active
            && self.next.is_none()
            && (transmit || receive)
            && self.status & 1 == 0
            && self.gate
        {
            self.phase = Phase::Load;
            self.next = Some(now()?.wait(1, Tap::cpu(), clocks)?);
        }
        Ok(())
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.gate == gate {
            return Ok(());
        }
        if let Some(wait) = &mut self.next {
            if gate {
                wait.resume(now, clocks)?;
            } else {
                wait.pause(now, clocks)?;
            }
        }
        self.gate = gate;
        self.schedule_load(now, clocks)
    }
    pub fn interrupt(&self) -> bool {
        self.interrupt_with_enable(0)
    }
    pub(crate) fn interrupt_with_enable(&self, retained: u8) -> bool {
        let enable = self.enable | retained;
        (self.status & 8 != 0 && enable & 8 != 0)
            || (self.status & 4 != 0 && enable & 4 != 0)
            || (self.status & 0x42 != 0 && enable & 2 != 0)
            || (self.status & 1 != 0 && enable & 1 != 0)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0xf0e0 => self.high | u8::from(self.mosi) << 4,
            0xf0e1 => self.low,
            0xf0e2 => self.mode,
            0xf0e3 => self.enable,
            0xf0e4 => self.status,
            0xf0e9 => self.rdr,
            0xf0eb => self.tdr,
            _ => 0,
        }
    }
    /// Reading SSSR qualifies the flags currently visible for later clearing.
    pub(crate) fn read_status(&mut self) -> u8 {
        self.seen = self.status;
        self.status
    }
    pub fn read(&mut self, address: u16, now: Time, clocks: &Clocks) -> Result<u8, Error> {
        self.read_at(address, || Ok(now), clocks)
    }
    /// Retained observations qualify flags without consuming a clock. Starting
    /// reception obtains the access timestamp before changing the owner state.
    pub(crate) fn read_at(
        &mut self,
        address: u16,
        now: impl FnOnce() -> Result<Time, Error>,
        clocks: &Clocks,
    ) -> Result<u8, Error> {
        let receive_at = if self.read_rearms(address) {
            Some(now()?)
        } else {
            None
        };
        let v = if address == 0xf0e4 {
            self.read_status()
        } else {
            self.peek(address)
        };
        if address == 0xf0e9 {
            self.status &= !2;
            self.seen &= !2;
            if let Some(now) = receive_at {
                self.receive_started = true;
                self.schedule_load(now, clocks)?;
            }
        }
        Ok(v)
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        mov: bool,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        self.write_at(address, value, mov, || Ok(Instant::time(now)), clocks)
    }
    /// Data and flag writes usually leave the current frame scheduled. Obtain
    /// a timestamp only when the owner actually changes a clock obligation.
    pub(crate) fn write_at(
        &mut self,
        address: u16,
        value: u8,
        mov: bool,
        mut now: impl FnMut() -> Result<Instant, Error>,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        match address {
            0xf0e0 => {
                let was_master = self.master();
                // The documented SOLP 0->1 transition can still change SOL.
                if mov && (value & 8 == 0 || !self.sol_protected) {
                    self.mosi = value & 0x10 != 0;
                }
                self.sol_protected = value & 8 != 0;
                self.high = (value & 0xe7) | 8;
                if self.active && was_master != self.master() {
                    self.next = if self.master() {
                        Some(now()?.wait(1, self.half_period(), clocks)?)
                    } else {
                        None
                    };
                    if !self.gate {
                        if let Some(wait) = &mut self.next {
                            wait.pause(now()?.time, clocks)?;
                        }
                    }
                }
            }
            0xf0e1 => {
                self.low = value & 0x58;
                if value & 0x20 != 0 {
                    self.next = None;
                    self.edges = 0;
                    self.sampled = 0;
                    self.shifted = 0;
                    self.receive_started = false;
                    self.active = false;
                    self.select_active = false;
                    self.clock_high = self.idle_high();
                }
            }
            0xf0e2 => {
                let changed = self.mode ^ value;
                self.mode = value & 0xe7;
                let tap = self.half_period();
                if let (Phase::Edge, Some(next)) = (self.phase, self.next.as_mut()) {
                    // CKS selects a live prescaler output. Retain the shifter
                    // and remaining half-edge obligation when it changes.
                    next.select(now()?.time, tap, clocks)?;
                    if changed & 0x40 != 0 {
                        self.clock_high = !self.clock_high;
                    }
                } else {
                    self.clock_high = self.idle_high();
                }
            }
            0xf0e3 => {
                self.enable = value & 0xef;
                if self.enable & 0x80 == 0 {
                    self.status |= 4;
                }
                if self.enable & 0x40 == 0 {
                    self.receive_started = false;
                }
                if self.enable & 0xc0 == 0 {
                    self.next = None;
                    self.active = false;
                    self.select_active = false;
                    self.clock_high = self.idle_high();
                }
            }
            0xf0e4 => {
                self.status &= !(self.seen & !value & 0x4f);
                self.seen &= value;
                if self.enable & 0x80 == 0 {
                    self.status |= 4;
                }
            }
            0xf0eb => {
                self.tdr = value;
                self.status &= !12;
                self.seen &= !12;
            }
            0xf0e9 => {}
            _ => {
                return Err(Error::Unmapped {
                    address,
                    write: true,
                    width: 1,
                })
            }
        }
        self.schedule_load_with(now, clocks)
    }
    /// Produce one clock edge, or perform the distinct holding-to-shift load.
    /// If sampling is requested, the caller resolves attached-device drivers
    /// and calls `sample` at this same timestamp.
    pub fn advance(&mut self, now: Time, clocks: &Clocks) -> Result<Option<Edge>, Error> {
        self.advance_at(Instant::time(now), clocks)
    }
    pub(crate) fn advance_at(
        &mut self,
        at: Instant,
        clocks: &Clocks,
    ) -> Result<Option<Edge>, Error> {
        let now = at.time;
        if self.deadline(clocks)? != Some(now) {
            return Err(Error::Internal("SSU event at wrong timestamp"));
        }
        match self.phase {
            Phase::Load => {
                self.next = None;
                if self.master()
                    && self.four_line()
                    && self.high & 2 != 0
                    && !self.select_active
                    && !self.select_high
                {
                    self.conflict();
                    return Ok(None);
                }
                self.transmitting = self.enable & 0x80 != 0 && self.status & 4 == 0;
                if self.transmitting {
                    self.tx = self.tdr;
                    self.status |= 4;
                }
                self.active = true;
                self.select_active = self.master();
                self.msb_first = self.mode & 0x80 != 0;
                self.rx = 0;
                self.edges = 0;
                self.shifted = 0;
                self.sampled = 0;
                self.clock_high = self.idle_high();
                if self.first_edge_samples() && self.transmitting {
                    self.mosi = self.output_bit(0);
                    self.shifted = 1;
                }
                self.phase = Phase::Edge;
                if self.master() {
                    self.next = Some(at.wait(1, self.half_period(), clocks)?);
                }
                Ok(None)
            }
            Phase::Edge => {
                self.clock_high = !self.clock_high;
                let edge = self.shift_edge();
                if self.edges < 16 {
                    self.next
                        .as_mut()
                        .ok_or(Error::Internal("SSU edge without clock obligation"))?
                        .follow(clocks)?;
                } else {
                    self.next = None;
                }
                Ok(Some(edge))
            }
        }
    }
    fn conflict(&mut self) {
        self.status |= 1;
        self.high &= !0x80;
        self.active = false;
        self.select_active = false;
        self.next = None;
    }
    fn shift_edge(&mut self) -> Edge {
        let sample = (self.edges & 1 == 0) == self.first_edge_samples();
        if !sample && self.shifted < 8 && self.transmitting {
            self.mosi = self.output_bit(self.shifted);
            self.shifted += 1;
        }
        self.edges += 1;
        Edge {
            clock: self.clock_high,
            mosi: self.mosi,
            sample,
        }
    }
    /// Observe resolved package levels. External and internal clocks enter the
    /// same shifter; selection changes cannot complete or discard a whole byte.
    pub fn input_pins(&mut self, select_high: bool, clock: bool) -> Option<Edge> {
        let deselected = !self.select_high && select_high;
        let changed = self.external_clock != clock;
        self.select_high = select_high;
        self.external_clock = clock;
        if !self.gate || self.master() || self.high & 4 == 0 {
            return None;
        }
        if self.four_line() && self.high & 3 != 0 && deselected && self.active && self.edges != 0 {
            self.conflict();
        }
        if self.active
            && self.selected()
            && changed
            && (self.edges != 0 || clock != self.idle_high())
        {
            self.clock_high = clock;
            Some(self.shift_edge())
        } else {
            None
        }
    }
    pub fn sample(&mut self, high: bool) {
        if self.sampled >= 8 {
            return;
        }
        if self.msb_first {
            self.rx = self.rx << 1 | u8::from(high);
        } else {
            self.rx |= u8::from(high) << self.sampled;
        }
        self.sampled += 1;
        if self.sampled == 8 {
            self.finish_sample();
        }
    }
    fn finish_sample(&mut self) {
        if self.enable & 0x40 != 0 && self.status & 0x40 == 0 {
            if self.status & 2 != 0 {
                self.status |= 0x40;
            } else {
                self.rdr = self.rx;
                self.status |= 2;
                self.received = self.received.wrapping_add(1);
            }
        }
    }
    pub fn finish_edge(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        self.finish_edge_at(Instant::time(now), clocks)
    }
    pub(crate) fn finish_edge_at(&mut self, now: Instant, clocks: &Clocks) -> Result<(), Error> {
        if self.finishing() {
            self.active = false;
            if self.transmitting && self.enable & 0x80 != 0 {
                self.transmitted = self.transmitted.wrapping_add(1);
                if self.status & 4 != 0 {
                    self.status |= 8;
                }
            }
            if self.enable & 0x20 != 0 {
                self.receive_started = false;
            }
            self.schedule_load_at(now, clocks)?;
            if self.next.is_none() {
                self.select_active = false;
            }
        }
        Ok(())
    }
}

impl Ssu {
    pub(crate) fn validate(&self, faulted: bool) -> Result<(), Error> {
        crate::state::require(
            self.edges <= 16
                && self.high & 0x18 == 8
                && self.status & !0x4f == 0
                && self.seen & !0x4f == 0
                && self.shifted <= 8
                && self.sampled <= 8
                && (faulted || !self.active || self.edges < 16)
                && (self.next.is_none() || self.phase != Phase::Edge || self.edges < 16)
                && self.low & !0x58 == 0
                && self.mode & !0xe7 == 0
                && self.enable & !0xef == 0,
            "invalid SSU shift progress",
        )?;
        if let Some(w) = self.next {
            w.validate()?;
        }
        Ok(())
    }
}

/// An event time, with the CPU clock ordinal of the edge it falls on when
/// known, which places clock waits without converting the time back.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Instant {
    pub time: Time,
    pub cpu: Option<u64>,
}
impl Instant {
    pub fn time(time: Time) -> Self {
        Self { time, cpu: None }
    }
    fn wait(self, edges: u64, tap: Tap, clocks: &Clocks) -> Result<ClockWait, Error> {
        if let Some(wait) = self
            .cpu
            .and_then(|cpu| ClockWait::after_cpu(cpu, edges, tap, clocks))
        {
            return wait;
        }
        ClockWait::after(self.time, edges, tap, clocks)
    }
}
