//! SSU holding registers and exact clock-edge sequences. The board resolves
//! their package drivers and attached devices before sampling the shifter.
use super::clocks::{ClockWait, Clocks, Tap};
use crate::{
    error::Error,
    serial::{mask, spread, Bits, Drives},
    signals::Drive,
    time::Time,
};
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
pub(crate) struct Prefix {
    pub count: u8,
    pub clock: u16,
    pub mosi: u16,
    pub samples: u16,
    launches: u8,
    pub last: Time,
    next: ClockWait,
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
    pub(crate) fn reads_shift(address: u16) -> bool {
        matches!(address, 0xf0e0 | 0xf0e9)
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
        Pins {
            drives: self
                .pin_planes(u16::from(self.clock_high), u16::from(self.mosi), 1)
                .map(|drive| drive.map(|drive| drive.at(0))),
            data_open_drain: self.high & 0x20 != 0,
        }
    }
    /// Project the live package drivers over exact chronological edge lanes.
    /// The ordinary pin projection is the one-lane case of this same rule.
    pub(crate) fn pin_planes(&self, clock: u16, mosi: u16, lanes: u16) -> [Option<Drives>; 4] {
        let mut pins = [None; 4];
        if !self.gate {
            return pins;
        }
        if self.four_line() && self.high & 3 != 0 {
            pins[0] = Some(Drives::constant(
                if self.master() && self.high & 2 != 0 && self.select_active {
                    Drive::Low
                } else {
                    Drive::Floating
                },
                lanes,
            ));
        }
        if self.high & 4 != 0 {
            pins[1] = Some(if self.master() {
                Drives::levels(clock, lanes, self.low & 0x10 != 0)
            } else {
                Drives::default()
            });
        }
        if self.enable & 0x40 != 0 {
            pins[self.input_pin()] = Some(Drives::default());
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
            pins[output] = Some(if enabled {
                Drives::levels(mosi, lanes, self.high & 0x20 != 0)
            } else {
                Drives::default()
            });
        }
        pins
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
            self.next = Some(ClockWait::after(now, 1, Tap::cpu(), clocks)?);
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
        let receive_at = if address == 0xf0e9 && self.enable & 0xc0 == 0x40 {
            Some(now()?)
        } else {
            None
        };
        let v = self.peek(address);
        if address == 0xf0e4 {
            self.seen = v;
        }
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
                        Some(ClockWait::after(now, 1, self.half_period(), clocks)?)
                    } else {
                        None
                    };
                    if !self.gate {
                        if let Some(wait) = &mut self.next {
                            wait.pause(now, clocks)?;
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
                    next.select(now, tap, clocks)?;
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
        self.schedule_load(now, clocks)
    }
    /// Produce one clock edge, or perform the distinct holding-to-shift load.
    /// If sampling is requested, the caller resolves attached-device drivers
    /// and calls `sample` at this same timestamp.
    pub fn advance(&mut self, now: Time, clocks: &Clocks) -> Result<Option<Edge>, Error> {
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
                    self.next = Some(ClockWait::after(now, 1, self.half_period(), clocks)?);
                }
                Ok(None)
            }
            Phase::Edge => {
                self.clock_high = !self.clock_high;
                let edge = self.shift_edge();
                self.next = if self.edges < 16 {
                    Some(
                        self.next
                            .ok_or(Error::Internal("SSU edge without clock obligation"))?
                            .following_edge(clocks)?,
                    )
                } else {
                    None
                };
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
    fn shift_plan(&self, count: u8) -> (u16, u16, u8) {
        let lanes = mask(count);
        let sample_first = (self.edges & 1 == 0) == self.first_edge_samples();
        let samples = (if sample_first { 0x5555 } else { 0xaaaa }) & lanes;
        let launches = (!samples & lanes).count_ones() as u8;
        let consumed = launches.min(8 - self.shifted);
        let leading = u8::from(sample_first);
        let mosi = if !self.transmitting || consumed == 0 {
            if self.mosi {
                lanes
            } else {
                0
            }
        } else {
            let byte = if self.msb_first {
                self.tx
            } else {
                self.tx.reverse_bits()
            };
            let bits = Bits::output(byte, self.shifted, consumed).value;
            let repeated = spread(bits);
            let high = (repeated | repeated << 1) << leading;
            let through = (leading + 2 * consumed).min(count);
            let mut mosi = high & mask(through);
            if sample_first && self.mosi {
                mosi |= 1;
            }
            if bits & (1 << (consumed - 1)) != 0 {
                mosi |= lanes & !mask(through);
            }
            mosi
        };
        (mosi, samples, if self.transmitting { consumed } else { 0 })
    }
    fn commit_shift(&mut self, count: u8, mosi: u16, launches: u8) {
        self.mosi = mosi & (1 << (count - 1)) != 0;
        self.shifted += launches;
        self.edges += count;
    }
    fn shift_edge(&mut self) -> Edge {
        let (mosi, samples, launches) = self.shift_plan(1);
        self.commit_shift(1, mosi, launches);
        Edge {
            clock: self.clock_high,
            mosi: self.mosi,
            sample: samples != 0,
        }
    }
    pub(crate) fn quiet_prefix(
        &self,
        end: Time,
        max: u8,
        clocks: &Clocks,
    ) -> Result<Option<Prefix>, Error> {
        if self.phase != Phase::Edge || !self.master() || !self.active {
            return Ok(None);
        }
        let Some(next) = self.next else {
            return Ok(None);
        };
        let sample = if self.first_edge_samples() { 15u8 } else { 16 };
        let sample_first = (self.edges & 1 == 0) == self.first_edge_samples();
        let before_sample = 2 * (7u8.saturating_sub(self.sampled)) + u8::from(!sample_first);
        let count = next.edges_before(end, clocks).min(u64::from(
            max.min(sample.saturating_sub(self.edges).saturating_sub(1))
                .min(15 - self.edges)
                .min(before_sample),
        )) as u8;
        if count < 2 {
            return Ok(None);
        }
        let (mosi, samples, launches) = self.shift_plan(count);
        let clock = (if self.clock_high { 0xaaaa } else { 0x5555 }) & mask(count);
        // Leave projection failures to one-edge retirement, which preserves
        // the exact fault prefix if the following obligation overflows.
        let last = match next.later_edge(u64::from(count - 1), clocks) {
            Ok(Some(last)) => last,
            _ => return Ok(None),
        };
        let next = match next.following_count(u64::from(count), clocks) {
            Ok(next) => next,
            Err(_) => return Ok(None),
        };
        Ok(Some(Prefix {
            count,
            clock,
            mosi,
            samples,
            launches,
            last,
            next,
        }))
    }
    pub(crate) fn commit_prefix(&mut self, prefix: &Prefix, samples: Bits) {
        self.commit_shift(prefix.count, prefix.mosi, prefix.launches);
        self.clock_high = prefix.clock & (1 << (prefix.count - 1)) != 0;
        self.next = Some(prefix.next);
        self.sample_bits(samples);
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
        self.sample_bits(Bits::one(high));
    }
    fn sample_bits(&mut self, bits: Bits) {
        if self.sampled >= 8 || bits.count == 0 {
            return;
        }
        if self.msb_first {
            bits.append(&mut self.rx, &mut self.sampled);
        } else {
            self.rx |= bits.value << self.sampled;
            self.sampled += bits.count;
        }
        if self.sampled == 8 && self.enable & 0x40 != 0 && self.status & 0x40 == 0 {
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
        if self.active && self.edges == 16 && self.next.is_none() {
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
            self.schedule_load(now, clocks)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_external_frame_cannot_load_as_an_active_shifter() {
        let mut s = Ssu {
            active: true,
            phase: Phase::Edge,
            edges: 16,
            next: None,
            ..Ssu::default()
        };
        assert!(s.validate(false).is_err());
        // A fault may stop board resolution before the final edge is retired.
        s.validate(true).unwrap();
        s.active = false;
        s.validate(false).unwrap();
    }
    #[test]
    fn holding_shift_and_receive_are_distinct() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut s = Ssu::default();
        s.set_gate(true, Time::ZERO, &c).unwrap();
        for (a, v) in [
            (0xf0e0, 0x8c),
            (0xf0e1, 0x40),
            (0xf0e2, 0x86),
            (0xf0e3, 0xc0),
            (0xf0eb, 0x35),
        ] {
            s.write(a, v, true, Time::ZERO, &c).unwrap();
        }
        assert_eq!(s.status & 12, 0);
        let t = s.deadline(&c).unwrap().unwrap();
        let mut completed = t;
        assert!(s.advance(t, &c).unwrap().is_none());
        assert_eq!(s.status & 12, 4);
        while let Some(t) = s.deadline(&c).unwrap() {
            completed = t;
            if let Some(edge) = s.advance(t, &c).unwrap() {
                if edge.sample {
                    s.sample(edge.mosi);
                }
                s.finish_edge(t, &c).unwrap();
            }
        }
        assert_eq!(s.read(0xf0e9, completed, &c).unwrap(), 0x35);
        assert_eq!(s.status & 14, 12);
    }
    #[test]
    fn receive_start_uses_the_consuming_read_time_and_preserves_failed_observations() {
        let c = Clocks::new(
            Time::ZERO,
            super::super::clocks::Frequencies {
                main_hz: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        let mut s = Ssu::default();
        s.set_gate(true, Time::ZERO, &c).unwrap();
        s.write(0xf0e0, 0x8c, true, Time::ZERO, &c).unwrap();
        s.write(0xf0e3, 0x40, true, Time::ZERO, &c).unwrap();
        assert_eq!(s.deadline(&c).unwrap(), None);
        let before = s.clone();
        assert!(s
            .read_at(
                0xf0e9,
                || Err(Error::Internal("unavailable access time")),
                &c
            )
            .is_err());
        assert_eq!(s, before);
        assert_eq!(
            s.read_at(0xf0e9, || Ok(Time::from_micros(123)), &c)
                .unwrap(),
            0
        );
        assert_eq!(s.deadline(&c).unwrap(), Some(Time::from_micros(124)));
    }
}
