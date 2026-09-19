//! H8/38602R §18: two analog comparators with read-armed interrupt latches.
//!
//! The default 15 µs response uses the manual's maximum conversion time.
//! See docs/accuracy/comparators.md for the basis of this timing choice.
use crate::{
    error::Error,
    time::{Duration, Time, TimeError},
};

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq, Default)]
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
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Comparators {
    channels: [Channel; 2],
    gate: bool,
    supply_mv: u16,
    reference_mv: u16,
    input_mv: [u16; 2],
    response: Duration,
    synchronized_at: Time,
    unpowered_since: Option<Time>,
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
            unpowered_since: None,
        }
    }
}
impl Comparators {
    pub(crate) fn set_supply(&mut self, on: bool, now: Time) -> Result<(), Error> {
        if on {
            if let Some(at) = self.unpowered_since.take() {
                let elapsed = now.duration_since(at).ok_or(TimeError::Reversed)?;
                for c in &mut self.channels {
                    c.due = c
                        .due
                        .map(|due| due.checked_add(elapsed).ok_or(TimeError::Overflow))
                        .transpose()?;
                }
                self.synchronized_at = now;
            }
        } else if self.unpowered_since.is_none() {
            self.sync(now)?;
            self.unpowered_since = Some(now);
        }
        Ok(())
    }
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
        let unpowered_since = self.unpowered_since;
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
            unpowered_since,
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
        // Table18.2/Fig18.2 and the internal-reference application note select
        // VIH without hysteresis, resolving the contrary CRS prose on p.363.
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
        self.gate = gate;
        for i in 0..2 {
            self.reevaluate(i, now, gate)?;
        }
        Ok(())
    }
    pub fn deadline(&self) -> Option<Time> {
        if self.unpowered_since.is_some() {
            return None;
        }
        self.channels.iter().filter_map(|c| c.due).min()
    }
    pub fn sync(&mut self, now: Time) -> Result<(), Error> {
        if self.unpowered_since.is_some() {
            return Ok(());
        }
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
    pub fn interrupt(&self) -> Option<u8> {
        self.interrupt_with_enable([0; 2])
    }
    pub(crate) fn interrupt_with_enable(&self, retained: [u8; 2]) -> Option<u8> {
        if !self.gate {
            return None;
        }
        self.channels
            .iter()
            .zip(retained)
            .enumerate()
            .find_map(|(i, (c, old))| {
                (c.control & 0x80 != 0 && (c.control | old) & 0x40 != 0 && c.flag)
                    .then_some(21 + i as u8)
            })
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
                let i = usize::from(address - 0xf0dc);
                let c = &mut self.channels[i];
                // CMR bypasses the internal ladder. Its CMLS/CRS bits remain
                // stored, but cannot postpone a live external comparison.
                let changed = (c.control ^ value) & 0xa0 != 0
                    || (value & 0x20 == 0 && (c.control ^ value) & 0x1f != 0);
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

impl Comparators {
    pub(crate) fn validate(&self, now: Time) -> Result<(), Error> {
        crate::state::require(
            self.synchronized_at <= now
                && self.unpowered_since.is_none_or(|at| at <= now)
                && self.response != Duration::ZERO
                && self
                    .channels
                    .iter()
                    .all(|c| c.event_at.is_none_or(|at| at <= now)),
            "invalid comparator timing",
        )
    }
}
