//! SCI3 asynchronous shift/holding registers and IrDA pulse output.
//! Receive inputs are edges, never complete bytes. Synchronous/external-clock
//! modes are explicit implementation boundaries in this starter.
use super::clocks::{Clocks, Tap};
use crate::{
    error::Error,
    signals::{Event, Output},
    time::{Duration, Time, TimeError},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tx {
    frame: u16,
    bits: u8,
    at_bit: u8,
    next: Time,
    period: Duration,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rx {
    value: u8,
    bit: i8,
    parity: bool,
    error: u8,
    next: Time,
    period: Duration,
    pulse: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
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
    gate: bool,
    holding: Option<u8>,
    tx: Option<Tx>,
    rx: Option<Rx>,
    pulse_end: Option<Time>,
    emitting: bool,
    input_light: bool,
    pub transmitted: u64,
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
            gate: false,
            holding: None,
            tx: None,
            rx: None,
            pulse_end: None,
            emitting: false,
            input_light: false,
            transmitted: 0,
            received: 0,
        }
    }
}
impl Sci {
    pub fn handles(a: u16) -> bool {
        matches!(a, 0xff91 | 0xff98..=0xff9d | 0xffa6 | 0xffa7)
    }
    fn bits(&self) -> u8 {
        if self.smr & 4 != 0 {
            5
        } else if self.smr & 0x40 != 0 {
            7
        } else {
            8
        }
    }
    fn parity(&self) -> bool {
        self.smr & 0x20 != 0
    }
    fn period(&self, now: Time, c: &Clocks) -> Result<Duration, Error> {
        let n = u64::from(self.brr) + 1;
        let scale = if self.semr & 8 != 0 { 16 } else { 32 };
        let tap = match self.smr & 3 {
            0 => Tap::system(1),
            1 => Tap::watch(1),
            2 => Tap::system(16),
            _ => Tap::system(64),
        };
        let start = c.edge(c.ticks(now, tap), tap)?;
        Ok(c.after(now, n * scale, tap)?.duration_since(start).unwrap())
    }
    fn validate_active(&self) -> Result<(), Error> {
        if self.smr & 0x80 != 0 || self.scr & 2 != 0 {
            return Err(Error::Unsupported {
                component: "SCI3",
                detail: "synchronous/external-clock mode is not implemented",
                address: 0xff98,
            });
        }
        if self.ircr & 0x80 != 0 && self.semr & 8 != 0 {
            return Err(Error::Unsupported {
                component: "SCI3",
                detail: "IrDA requires ABCS=0",
                address: 0xffa6,
            });
        }
        Ok(())
    }
    fn emit(&mut self, at: Time, on: bool, out: &mut dyn Output) {
        if self.emitting != on {
            self.emitting = on;
            out.event(Event::Infrared { at, emitting: on });
        }
    }
    fn drive_bit(
        &mut self,
        now: Time,
        one: bool,
        period: Duration,
        c: &Clocks,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        self.pulse_end = None;
        if self.spcr & 0x10 == 0 {
            self.emit(now, false, out);
            return Ok(());
        }
        if self.ircr & 0x80 != 0 {
            let active = !one;
            self.emit(now, active ^ (self.spcr & 2 != 0), out);
            if active {
                let d = match self.ircr >> 4 & 7 {
                    0 => Duration::from_raw(period.raw() / 16 * 3),
                    n @ 1..=4 => c
                        .after(now, 1u64 << n, Tap::system(1))?
                        .duration_since(now)
                        .unwrap(),
                    _ => {
                        return Err(Error::Unsupported {
                            component: "SCI3",
                            detail: "prohibited IrDA pulse divisor",
                            address: 0xffa7,
                        })
                    }
                };
                self.pulse_end = Some(now.checked_add(d).ok_or(TimeError::Overflow)?);
            }
        } else {
            self.emit(now, (!one) ^ (self.spcr & 2 != 0), out);
        }
        Ok(())
    }
    fn start(&mut self, now: Time, c: &Clocks, out: &mut dyn Output) -> Result<(), Error> {
        if !self.gate || self.scr & 0x20 == 0 || self.tx.is_some() {
            return Ok(());
        }
        let Some(value) = self.holding.take() else {
            return Ok(());
        };
        self.validate_active()?;
        let n = self.bits();
        let mut frame = u16::from(value & ((1u16 << n) - 1) as u8) << 1;
        let mut length = n + 1;
        if self.parity() {
            let parity = (value & ((1u16 << n) - 1) as u8).count_ones() & 1 != 0;
            frame |= u16::from(parity ^ (self.smr & 0x10 != 0)) << length;
            length += 1;
        }
        frame |= 1 << length;
        length += 1;
        if self.smr & 8 != 0 {
            frame |= 1 << length;
            length += 1;
        }
        let period = self.period(now, c)?;
        self.tx = Some(Tx {
            frame,
            bits: length,
            at_bit: 0,
            next: now.checked_add(period).ok_or(TimeError::Overflow)?,
            period,
        });
        self.ssr |= 0x80;
        self.ssr &= !4;
        self.drive_bit(now, false, period, c, out)
    }
    pub fn deadline(&self) -> Option<Time> {
        if !self.gate {
            return None;
        }
        [
            self.tx.map(|s| s.next),
            self.rx.map(|s| s.next),
            self.pulse_end,
        ]
        .into_iter()
        .flatten()
        .min()
    }
    pub fn interrupt(&self) -> bool {
        self.gate
            && ((self.ssr & 0x80 != 0 && self.scr & 0x80 != 0)
                || (self.ssr & 0x78 != 0 && self.scr & 0x40 != 0)
                || (self.ssr & 4 != 0 && self.scr & 4 != 0))
    }
    pub fn set_gate(&mut self, gate: bool, now: Time, out: &mut dyn Output) {
        if self.gate && !gate {
            self.emit(now, false, out);
            let light = self.input_light;
            *self = Self::default();
            self.input_light = light;
        }
        self.gate = gate;
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
        let v = self.peek(a);
        if a == 0xff9c {
            self.seen = v;
        }
        if a == 0xff9d {
            self.ssr &= !0x40;
            self.seen &= !0x40;
        }
        v
    }
    pub fn write(
        &mut self,
        a: u16,
        v: u8,
        now: Time,
        c: &Clocks,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        if !self.gate {
            return Ok(());
        }
        match a {
            0xff91 => self.spcr = (v & 0x13) | 0xc0,
            0xff98 | 0xff99 | 0xffa6 | 0xffa7 => {
                if self.tx.is_some() || self.rx.is_some() {
                    return Err(Error::Unsupported {
                        component: "SCI3",
                        detail: "format/baud change during frame",
                        address: a,
                    });
                }
                match a {
                    0xff98 => self.smr = v,
                    0xff99 => self.brr = v,
                    0xffa6 => self.semr = v & 8,
                    _ => self.ircr = v & 0xf0,
                }
            }
            0xff9a => {
                self.scr = v & 0xf7;
                if v & 0x20 == 0 {
                    self.tx = None;
                    self.holding = None;
                    self.pulse_end = None;
                    self.ssr |= 0x84;
                    self.emit(now, false, out);
                }
                if v & 0x10 == 0 {
                    self.rx = None;
                }
                if v & 0x30 != 0 {
                    self.validate_active()?;
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
                if self.ssr & 0x80 == 0 && self.scr & 0x20 != 0 && self.holding.is_none() {
                    self.holding = Some(self.tdr);
                    self.ssr &= !4;
                }
                if self.scr & 0x20 == 0 {
                    self.ssr |= 0x80;
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
        self.start(now, c, out)
    }
    fn line(&self) -> bool {
        !self.input_light ^ (self.spcr & 1 != 0)
    }
    pub fn receive_light(&mut self, light: bool, now: Time, c: &Clocks) -> Result<(), Error> {
        let old = self.line();
        let old_light = self.input_light;
        self.input_light = light;
        if !self.gate || self.scr & 0x10 == 0 {
            return Ok(());
        }
        let edge = if self.ircr & 0x80 != 0 {
            !old_light && light
        } else {
            old && !self.line()
        };
        if edge {
            if let Some(ref mut rx) = self.rx {
                if self.ircr & 0x80 != 0 {
                    rx.pulse = true;
                }
            } else {
                self.validate_active()?;
                let period = self.period(now, c)?;
                self.rx = Some(Rx {
                    value: 0,
                    bit: -1,
                    parity: false,
                    error: 0,
                    next: now
                        .checked_add(Duration::from_raw(period.raw() / 2))
                        .ok_or(TimeError::Overflow)?,
                    period,
                    pulse: true,
                });
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, now: Time, c: &Clocks, out: &mut dyn Output) -> Result<(), Error> {
        if self.pulse_end == Some(now) {
            self.pulse_end = None;
            self.emit(now, self.spcr & 2 != 0, out);
        }
        if let Some(mut tx) = self.tx {
            if tx.next == now {
                tx.at_bit += 1;
                if tx.at_bit == tx.bits {
                    self.tx = None;
                    self.transmitted = self.transmitted.wrapping_add(1);
                    if self.holding.is_none() {
                        self.ssr |= 4;
                        self.drive_bit(now, true, tx.period, c, out)?;
                    }
                    self.start(now, c, out)?;
                } else {
                    tx.next = now.checked_add(tx.period).ok_or(TimeError::Overflow)?;
                    self.tx = Some(tx);
                    self.drive_bit(now, tx.frame & (1 << tx.at_bit) != 0, tx.period, c, out)?;
                }
            }
        }
        if let Some(mut rx) = self.rx {
            if rx.next == now {
                let high = if self.ircr & 0x80 != 0 {
                    !rx.pulse
                } else {
                    self.line()
                };
                rx.pulse = false;
                if rx.bit < 0 && high {
                    self.rx = None;
                    return Ok(());
                }
                let n = self.bits() as i8;
                if (0..n).contains(&rx.bit) {
                    if high {
                        rx.value |= 1 << rx.bit;
                        rx.parity = !rx.parity;
                    }
                } else if rx.bit == n && self.parity() {
                    if high != (rx.parity ^ (self.smr & 0x10 != 0)) {
                        rx.error |= 8;
                    }
                } else if rx.bit >= n + i8::from(self.parity()) {
                    if !high {
                        rx.error |= 0x10;
                    }
                    if self.ssr & 0x40 != 0 {
                        rx.error |= 0x20;
                    }
                    self.ssr |= rx.error;
                    if rx.error == 0 {
                        self.rdr = rx.value;
                        self.ssr |= 0x40;
                        self.received = self.received.wrapping_add(1);
                    }
                    self.rx = None;
                    return Ok(());
                }
                rx.bit += 1;
                rx.next = now.checked_add(rx.period).ok_or(TimeError::Overflow)?;
                self.rx = Some(rx);
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transmitter_produces_timed_pulses_and_keeps_holding_distinct() {
        let c = Clocks::new(Time::ZERO, Default::default()).unwrap();
        let mut s = Sci::default();
        let mut events = vec![];
        s.set_gate(true, Time::ZERO, &mut events);
        for (a, v) in [
            (0xff91, 0xd0),
            (0xff99, 1),
            (0xffa7, 0x80),
            (0xff9a, 0x20),
            (0xff9b, 0xa5),
        ] {
            s.write(a, v, Time::ZERO, &c, &mut events).unwrap();
        }
        assert_eq!(s.ssr & 0x84, 0x80);
        while let Some(t) = s.deadline() {
            s.advance(t, &c, &mut events).unwrap();
        }
        assert_eq!(s.transmitted, 1);
        assert_eq!(s.ssr & 0x84, 0x84);
        assert_eq!(events.len(), 10);
    }
}
