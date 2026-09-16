//! H8/38602R §18: two analog comparators with read-armed interrupt latches.
//!
//! The digital register/latch contract is documented. The default 15 µs
//! inertial response is a *bounded timing witness*: §21 gives a maximum, not
//! a measured delay for a particular unit. No silicon noise/offset is claimed.
use crate::{
    error::Error,
    time::{Duration, Time, TimeError},
};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Channel {
    control: u8,
    result: bool,
    baseline: bool,
    armed: bool,
    flag: bool,
    seen: bool,
    target: bool,
    settling: bool,
    due: Option<Time>,
    // A read strobe masks the same-time comparison interrupt (§18.4.3).
    event_at: Option<Time>,
    flag_before_event: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparators {
    channels: [Channel; 2],
    gate: bool,
    supply_mv: u16,
    reference_mv: u16,
    input_mv: [u16; 2],
    response: Duration,
    synchronized_at: Time,
}
impl Default for Comparators {
    fn default() -> Self {
        Self {
            channels: Default::default(),
            gate: false,
            supply_mv: 3000,
            reference_mv: 0,
            input_mv: [0; 2],
            response: Duration::from_micros(15),
            synchronized_at: Time::ZERO,
        }
    }
}
impl Comparators {
    pub fn with_response(response: Duration) -> Result<Self, Error> {
        if response == Duration::ZERO {
            return Err(Error::BadInput("comparator response must be positive"));
        }
        Ok(Self {
            response,
            ..Default::default()
        })
    }
    pub fn reset(&mut self, now: Time) {
        let response = self.response;
        let input_mv = self.input_mv;
        let supply_mv = self.supply_mv;
        let reference_mv = self.reference_mv;
        *self = Self {
            response,
            input_mv,
            supply_mv,
            reference_mv,
            synchronized_at: now,
            ..Default::default()
        };
    }
    pub fn enabled_mask(&self) -> u8 {
        self.channels.iter().enumerate().fold(0, |mask, (i, c)| {
            mask | (u8::from(self.gate && c.control & 0x80 != 0) << i)
        })
    }
    pub fn uses_external_reference(&self) -> bool {
        self.gate && self.channels.iter().any(|c| c.control & 0xa0 == 0xa0)
    }
    fn desired(&self, i: usize) -> bool {
        let c = &self.channels[i];
        if c.control & 0x20 != 0 {
            return self.input_mv[i] > self.reference_mv;
        }
        // Table 18.2 and Fig.18.2 explicitly select VIH without hysteresis.
        // The CRS prose on p.363 says VIL instead; that inconsistency is retained
        // in the evidence notes. Do not present its resolution as a capture.
        let n = if c.control & 0x10 != 0 && c.result {
            9
        } else {
            11
        } + u32::from(c.control & 15);
        u32::from(self.input_mv[i]) * 30 > u32::from(self.supply_mv) * n
    }
    fn reevaluate(&mut self, i: usize, now: Time, restart: bool) -> Result<(), Error> {
        if !self.gate || self.channels[i].control & 0x80 == 0 {
            self.channels[i].due = None;
            self.channels[i].settling = false;
            return Ok(());
        }
        let desired = self.desired(i);
        let c = &mut self.channels[i];
        if restart {
            c.target = desired;
            c.settling = true;
            c.due = Some(now.checked_add(self.response).ok_or(TimeError::Overflow)?);
        } else if desired == c.result && !c.settling {
            c.due = None;
        } else if c.due.is_none() || c.target != desired {
            c.target = desired;
            c.due = Some(now.checked_add(self.response).ok_or(TimeError::Overflow)?);
        }
        Ok(())
    }
    pub fn set_inputs(
        &mut self,
        now: Time,
        supply_mv: u16,
        reference_mv: u16,
        input_mv: [u16; 2],
    ) -> Result<(), Error> {
        self.sync(now)?;
        if (self.supply_mv, self.reference_mv, self.input_mv) == (supply_mv, reference_mv, input_mv)
        {
            return Ok(());
        }
        self.supply_mv = supply_mv;
        self.reference_mv = reference_mv;
        self.input_mv = input_mv;
        for i in 0..2 {
            self.reevaluate(i, now, false)?;
        }
        Ok(())
    }
    pub fn set_gate(&mut self, gate: bool, now: Time) -> Result<(), Error> {
        if self.gate == gate {
            return Ok(());
        }
        self.sync(now)?;
        if !gate && self.channels.iter().any(|c| c.control & 0x80 != 0) {
            return Err(Error::Unsupported {
                component: "comparators",
                detail: "clear CME before module standby (§18.5)",
                address: 0xfffb,
            });
        }
        self.gate = gate;
        for i in 0..2 {
            self.reevaluate(i, now, gate)?;
        }
        Ok(())
    }
    pub fn deadline(&self) -> Option<Time> {
        self.channels.iter().filter_map(|c| c.due).min()
    }
    pub fn sync(&mut self, now: Time) -> Result<(), Error> {
        if now < self.synchronized_at {
            return Err(TimeError::Reversed.into());
        }
        self.synchronized_at = now;
        for c in &mut self.channels {
            if let Some(due) = c.due {
                if due <= now {
                    c.due = None;
                    c.settling = false;
                    c.result = c.target;
                    c.event_at = Some(due);
                    c.flag_before_event = c.flag;
                    if c.armed && c.result != c.baseline {
                        c.flag = true;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn interrupt(&self) -> bool {
        self.gate
            && self
                .channels
                .iter()
                .any(|c| c.control & 0xc0 == 0xc0 && c.flag)
    }
    pub fn peek(&self, address: u16) -> u8 {
        match address {
            0xf0dc | 0xf0dd => self.channels[usize::from(address - 0xf0dc)].control,
            0xf0de => self.channels.iter().enumerate().fold(0, |v, (i, c)| {
                v | (u8::from(c.result) << i) | (u8::from(c.flag) << (i + 4))
            }),
            _ => 0,
        }
    }
    pub fn read(&mut self, address: u16) -> u8 {
        if address == 0xf0de {
            for c in &mut self.channels {
                if c.event_at == Some(self.synchronized_at) {
                    c.flag = c.flag_before_event;
                    c.event_at = None;
                }
                c.seen = c.flag;
                if self.gate && c.control & 0xc0 == 0xc0 {
                    c.baseline = c.result;
                    c.armed = true;
                }
            }
        }
        self.peek(address)
    }
    pub fn write(&mut self, address: u16, value: u8, now: Time) -> Result<(), Error> {
        self.sync(now)?;
        match address {
            0xf0dc | 0xf0dd => {
                if value & 0x30 == 0x30 {
                    return Err(Error::Unsupported {
                        component: "comparators",
                        detail: "external reference with hysteresis is prohibited",
                        address,
                    });
                }
                let i = usize::from(address - 0xf0dc);
                let c = &mut self.channels[i];
                let changed = (c.control ^ value) & 0xbf != 0;
                c.control = value;
                if value & 0xc0 != 0xc0 {
                    c.armed = false;
                }
                self.reevaluate(i, now, changed)?;
            }
            0xf0de => {
                for (i, c) in self.channels.iter_mut().enumerate() {
                    if c.seen && value & (0x10 << i) == 0 {
                        c.flag = false;
                        c.seen = false;
                    }
                }
            }
            _ => {
                return Err(Error::Unmapped {
                    address,
                    write: true,
                    width: 1,
                })
            }
        }
        Ok(())
    }
}
