//! SSU holding registers and one edge-level shifter. The board handles every
//! emitted edge; no completed-byte route into an attached device exists.
use super::clocks::{ClockWait, Clocks, Tap};
use crate::{error::Error, time::Time};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Load,
    Edge,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub clock: bool,
    pub mosi: bool,
    pub sample: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ssu {
    high: u8,
    low: u8,
    mode: u8,
    enable: u8,
    status: u8,
    seen: u8,
    tdr: u8,
    rdr: u8,
    holding: Option<u8>,
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
    pub transmitted: u64,
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
            holding: None,
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
    pub fn deadline(&self, clocks: &Clocks) -> Result<Option<Time>, Error> {
        self.next
            .as_ref()
            .map_or(Ok(None), |wait| wait.deadline(clocks))
    }
    pub fn pins(&self) -> Option<(bool, bool)> {
        (self.gate && self.high & 0x84 == 0x84 && self.enable & 0xc0 != 0)
            .then_some((self.clock_high, self.mosi))
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
        let shift = if self.mode & 0x80 != 0 {
            7 - index
        } else {
            index
        };
        self.tx & (1u8 << shift) != 0
    }
    fn schedule_load(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.next.is_none()
            && self.holding.is_some()
            && self.enable & 0x80 != 0
            && self.status & 0x40 == 0
            && self.gate
        {
            if self.high & 0x80 == 0 {
                return Err(Error::Unsupported {
                    component: "SSU",
                    detail: "slave-clock routing is not implemented",
                    address: 0xf0e0,
                });
            }
            if self.high & 0x40 != 0 {
                return Err(Error::Unsupported {
                    component: "SSU",
                    detail: "bidirectional turnaround is not implemented",
                    address: 0xf0e0,
                });
            }
            self.phase = Phase::Load;
            self.next = Some(ClockWait::after(now, 1, Tap::system(1), clocks)?);
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
        (self.status & 8 != 0 && self.enable & 8 != 0)
            || (self.status & 4 != 0 && self.enable & 4 != 0)
            || (self.status & 0x42 != 0 && self.enable & 2 != 0)
            || (self.status & 1 != 0 && self.enable & 1 != 0)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0xf0e0 => self.high,
            0xf0e1 => self.low,
            0xf0e2 => self.mode,
            0xf0e3 => self.enable,
            0xf0e4 => self.status,
            0xf0e9 => self.rdr,
            0xf0eb => self.tdr,
            _ => 0,
        }
    }
    pub fn read(&mut self, address: u16) -> u8 {
        let v = self.peek(address);
        if address == 0xf0e4 {
            self.seen = v;
        }
        if address == 0xf0e9 {
            self.status &= !2;
            self.seen &= !2;
        }
        v
    }
    pub fn write(
        &mut self,
        address: u16,
        value: u8,
        now: Time,
        clocks: &Clocks,
    ) -> Result<(), Error> {
        match address {
            0xf0e0 => {
                self.high = (value & 0xf7) | 8;
            }
            0xf0e1 => {
                self.low = value & 0x58;
                if value & 0x20 != 0 {
                    self.holding = None;
                    self.next = None;
                    self.edges = 0;
                    self.sampled = 0;
                    self.status = 4;
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
                if value & 0xc0 == 0x40 {
                    return Err(Error::Unsupported {
                        component: "SSU",
                        detail: "receive-only autonomous master sequencing is not implemented",
                        address,
                    });
                }
                self.enable = value & 0xef;
                if self.enable & 0x80 == 0 {
                    self.holding = None;
                    self.next = None;
                    self.status |= 4;
                }
                if self.enable & 0x40 == 0 {
                    self.status &= !0x42;
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
                if self.holding.is_some() {
                    self.status |= 1;
                    return Ok(());
                }
                self.holding = Some(value);
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
                self.tx = self
                    .holding
                    .take()
                    .ok_or(Error::Internal("SSU load without holding byte"))?;
                self.status |= 4;
                self.status &= !8;
                self.rx = 0;
                self.edges = 0;
                self.shifted = 0;
                self.sampled = 0;
                self.clock_high = self.idle_high();
                if self.first_edge_samples() {
                    self.mosi = self.output_bit(0);
                    self.shifted = 1;
                }
                self.phase = Phase::Edge;
                self.next = Some(ClockWait::after(now, 1, self.half_period(), clocks)?);
                Ok(None)
            }
            Phase::Edge => {
                self.clock_high = !self.clock_high;
                let first = self.edges & 1 == 0;
                let sample = first == self.first_edge_samples();
                if !sample && self.shifted < 8 {
                    self.mosi = self.output_bit(self.shifted);
                    self.shifted += 1;
                }
                self.edges += 1;
                self.next = if self.edges < 16 {
                    Some(ClockWait::after(now, 1, self.half_period(), clocks)?)
                } else {
                    None
                };
                Ok(Some(Edge {
                    clock: self.clock_high,
                    mosi: self.mosi,
                    sample,
                }))
            }
        }
    }
    pub fn sample(&mut self, high: bool) {
        if self.sampled >= 8 {
            return;
        }
        if self.mode & 0x80 != 0 {
            self.rx = self.rx << 1 | u8::from(high);
        } else {
            self.rx |= u8::from(high) << self.sampled;
        }
        self.sampled += 1;
        if self.sampled == 8 && self.enable & 0x40 != 0 {
            if self.status & 2 != 0 {
                self.status |= 0x40;
            } else {
                self.rdr = self.rx;
                self.status |= 2;
                self.received = self.received.wrapping_add(1);
            }
            if self.enable & 0x20 != 0 {
                self.enable &= !0x40;
            }
        }
    }
    pub fn finish_edge(&mut self, now: Time, clocks: &Clocks) -> Result<(), Error> {
        if self.edges == 16 && self.next.is_none() {
            self.transmitted = self.transmitted.wrapping_add(1);
            if self.holding.is_none() {
                self.status |= 8;
            }
            self.schedule_load(now, clocks)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
            s.write(a, v, Time::ZERO, &c).unwrap();
        }
        assert_eq!(s.status & 12, 0);
        let t = s.deadline(&c).unwrap().unwrap();
        assert!(s.advance(t, &c).unwrap().is_none());
        assert_eq!(s.status & 12, 4);
        while let Some(t) = s.deadline(&c).unwrap() {
            if let Some(edge) = s.advance(t, &c).unwrap() {
                if edge.sample {
                    s.sample(edge.mosi);
                }
                s.finish_edge(t, &c).unwrap();
            }
        }
        assert_eq!(s.read(0xf0e9), 0x35);
        assert_eq!(s.status & 14, 12);
    }
}
