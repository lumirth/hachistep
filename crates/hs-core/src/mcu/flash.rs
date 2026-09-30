//! H8/38606F flash: software-controlled pulses, persistent cell exposure and
//! a four-byte verify sense latch. See docs/accuracy/flash.md.
use super::{control::Mode, FLASH_SIZE};
use crate::{
    cpu::Width,
    error::Error,
    signals::{Event, NvDomain, Output},
    time::{Duration, Time, TimeError},
};

const PAGE: usize = 128;
const PAGES: usize = FLASH_SIZE / PAGE;
const SWE: u8 = 0x40;
const ESU: u8 = 0x20;
const PSU: u8 = 0x10;
const EV: u8 = 8;
const PV: u8 = 4;
const E: u8 = 2;
const P: u8 = 1;
// 7 ms of programming or 100 ms of erasing traverses the nominal charge range.
const FULL: u64 = (100 * ((7u128 << 64) / 1000)) as u64;
type Pulse = (bool, usize, usize, Time);

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
struct ChargePage {
    address: u16,
    cells: [u64; PAGE * 8],
}

#[derive(Debug, PartialEq, Eq)]
struct Charges {
    pages: Vec<ChargePage>,
    indices: [u16; PAGES],
}
impl Clone for Charges {
    fn clone(&self) -> Self {
        // Restoration must retain the no-allocation contract for later writes.
        let mut pages = Vec::with_capacity(PAGES);
        pages.extend_from_slice(&self.pages);
        Self {
            pages,
            indices: self.indices,
        }
    }
}
impl Charges {
    fn new() -> Self {
        Self {
            pages: Vec::with_capacity(PAGES),
            indices: [u16::MAX; PAGES],
        }
    }
    fn page(&mut self, address: usize, bytes: &[u8; FLASH_SIZE]) -> &mut ChargePage {
        let n = address / PAGE;
        let index = self.indices[n];
        let index = if index == u16::MAX {
            let index = self.pages.len();
            let mut page = ChargePage {
                address: address as u16,
                cells: [0; PAGE * 8],
            };
            for (i, q) in page.cells.iter_mut().enumerate() {
                if bytes[address + i / 8] & (1 << (i % 8)) == 0 {
                    *q = FULL;
                }
            }
            self.pages.push(page); // One reserved slot for every physical page.
            self.indices[n] = index as u16;
            index
        } else {
            usize::from(index)
        };
        &mut self.pages[index]
    }
    fn charge(&self, address: usize, bit: usize, byte: u8) -> u64 {
        let index = self.indices[address / PAGE];
        if index == u16::MAX {
            if byte & (1 << bit) == 0 {
                FULL
            } else {
                0
            }
        } else {
            self.pages[usize::from(index)].cells[(address % PAGE) * 8 + bit]
        }
    }
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Supply {
    Normal = 0,
    Reduced = 1,
    Stopped = 2,
}

/// Ordinary-read permission for an exclusive MCU interval. Its holder must
/// return before flash control, protection, supply or clock conditions change.
#[derive(Clone, Copy)]
pub(super) struct NormalRead(());

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Flash {
    #[borsh(deserialize_with = "crate::state::read_bytes")]
    bytes: Box<[u8; FLASH_SIZE]>,
    charges: Charges,
    control: u8,
    error: bool,
    erase: u8,
    enabled: bool,
    power_down_disabled: bool,
    latch: [u8; PAGE],
    page: u16,
    supply: Supply,
    oscillator: bool,
    mode: Mode,
    module: bool,
    ready: Time,
    swe_ready: Time,
    program_ready: Time,
    erase_ready: Time,
    recovery: Time,
    pulse_since: Time,
    verify_ready: Time,
    verify_pending: Option<(u16, Time)>,
    sensed: [u8; 4],
}
fn after(now: Time, micros: u64) -> Result<Time, Error> {
    now.checked_add(Duration::from_micros(micros))
        .ok_or(TimeError::Overflow.into())
}
fn sensed_bit(address: usize, bit: usize, charge: u64, mode: u8) -> bool {
    let i = address * 8 + bit;
    let class = ((i * 13) ^ (i >> 4)) & 15;
    let q = u128::from(charge);
    let full = u128::from(FULL);
    match mode {
        PV => q < full * (16 + class as u128) / 31,
        EV => q <= full * class as u128 / 256,
        _ => q < full * (16 + class as u128) / 62,
    }
}
impl Flash {
    pub fn new(firmware: &[u8]) -> Result<Self, Error> {
        if firmware.len() != FLASH_SIZE {
            return Err(Error::ImageSize {
                name: "firmware",
                expected: FLASH_SIZE,
                actual: firmware.len(),
            });
        }
        let bytes = firmware
            .to_vec()
            .into_boxed_slice()
            .try_into()
            .map_err(|_| Error::Internal("flash allocation shape"))?;
        Ok(Self {
            bytes,
            charges: Charges::new(),
            control: 0,
            error: false,
            erase: 0,
            enabled: false,
            power_down_disabled: false,
            latch: [0xff; PAGE],
            page: 0,
            supply: Supply::Normal,
            oscillator: true,
            mode: Mode::Active,
            module: true,
            ready: Time::ZERO,
            swe_ready: Time::ZERO,
            program_ready: Time::ZERO,
            erase_ready: Time::ZERO,
            recovery: Time::ZERO,
            pulse_since: Time::ZERO,
            verify_ready: Time::ZERO,
            verify_pending: None,
            sensed: [0xff; 4],
        })
    }
    pub fn handles(a: u16) -> bool {
        matches!(a, 0xf020..=0xf023 | 0xf02b)
    }
    pub fn register(&self, a: u16) -> u8 {
        if a == 0xf02b {
            return u8::from(self.enabled) << 7;
        }
        if !self.enabled {
            return 0;
        }
        match a {
            0xf020 => self.control,
            0xf021 => u8::from(self.error) << 7,
            0xf022 => u8::from(self.power_down_disabled) << 7,
            0xf023 => self.erase,
            _ => 0,
        }
    }
    pub fn write_register(
        &mut self,
        a: u16,
        value: u8,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        if a == 0xf02b {
            self.enabled = value & 0x80 != 0;
            return Ok(());
        }
        if !self.enabled {
            return Ok(());
        }
        let previous = self.pulse();
        self.settle(now, out);
        match a {
            0xf020 => {
                let value = if value & SWE != 0 { value & 0x7f } else { 0 };
                let rising = value & !self.control;
                let falling = self.control & !value;
                for (mask, micros) in [(P | PSU, 5), (E | ESU, 10), (PV, 2), (EV, 4), (SWE, 100)] {
                    if falling & mask != 0 {
                        self.recovery = self.recovery.max(after(now, micros)?);
                    }
                }
                if rising & SWE != 0 {
                    self.swe_ready = after(now, 1)?;
                }
                if rising & PSU != 0 {
                    self.program_ready = after(now, 50)?;
                }
                if rising & ESU != 0 {
                    self.erase_ready = after(now, 100)?;
                }
                if rising & (PV | EV) != 0 {
                    self.verify_ready = after(now, if value & EV != 0 { 20 } else { 4 })?;
                }
                if (self.control ^ value) & (P | E | PV | EV) != 0 {
                    self.verify_pending = None;
                }
                self.control = value;
                if value & SWE == 0 {
                    self.erase = 0;
                }
            }
            0xf022 => self.power_down_disabled = value & 0x80 != 0,
            0xf023 if self.control & SWE != 0 => {
                self.erase = if value.count_ones() <= 1 { value } else { 0 };
            }
            _ => {}
        }
        // A normal pulse ends by lowering P/E while retaining its setup bias.
        // Retargeting or abruptly removing the bias interrupts that pulse.
        let complete = a == 0xf020
            && previous.is_some_and(|(program, _, _, _)| {
                self.control & (SWE | PSU | ESU | 15) == SWE | if program { PSU } else { ESU }
            });
        self.finish_pulse(previous, now, complete, out);
        Ok(())
    }
    fn pulse(&self) -> Option<Pulse> {
        if self.error
            || !self.oscillator
            || self.supply != Supply::Normal
            || self.control & SWE == 0
        {
            return None;
        }
        let (program, start, end, ready) = match self.control & 15 {
            P if self.control & PSU != 0 => {
                let start = usize::from(self.page);
                (true, start, start + PAGE, self.program_ready)
            }
            E if self.control & ESU != 0 => {
                let (start, end) = match self.erase {
                    1 => (0, 0x400),
                    2 => (0x400, 0x800),
                    4 => (0x800, 0xc00),
                    8 => (0xc00, 0x1000),
                    16 => (0x1000, 0x8000),
                    32 => (0x8000, 0xc000),
                    _ => (0, 0),
                };
                (false, start, end, self.erase_ready)
            }
            _ => return None,
        };
        Some((
            program,
            start,
            end,
            ready.max(self.swe_ready).max(self.ready).max(self.recovery),
        ))
    }
    fn settle(&mut self, now: Time, out: &mut dyn Output) {
        if let Some((program, start, end, ready)) = self.pulse() {
            let elapsed = now.raw().saturating_sub(self.pulse_since.max(ready).raw());
            let amount = (elapsed.min(u128::from(FULL)) * if program { 100 } else { 7 })
                .min(u128::from(FULL)) as u64;
            if amount != 0 {
                for address in (start..end).step_by(PAGE) {
                    let page = self.charges.page(address, &self.bytes);
                    for byte in 0..PAGE {
                        let mut sensed = 0;
                        for bit in 0..8 {
                            let q = &mut page.cells[byte * 8 + bit];
                            if program {
                                if self.latch[byte] & (1 << bit) == 0 {
                                    *q = q.saturating_add(amount).min(FULL);
                                }
                            } else {
                                *q = q.saturating_sub(amount);
                            }
                            sensed |= u8::from(sensed_bit(address + byte, bit, *q, 0)) << bit;
                        }
                        if self.bytes[address + byte] != sensed {
                            self.bytes[address + byte] = sensed;
                            let _ = out.event(Event::NvByte {
                                at: now,
                                domain: NvDomain::InternalFlash,
                                address: (address + byte) as u16,
                                value: sensed,
                            });
                        }
                    }
                }
            }
        }
        self.pulse_since = now;
        if let Some((address, ready)) = self.verify_pending {
            if now >= ready && self.supply != Supply::Stopped {
                let mode = self.control & 15;
                if matches!(mode, PV | EV) {
                    for lane in 0..4 {
                        let address = usize::from(address) + lane;
                        let mut value = 0;
                        for bit in 0..8 {
                            let q = self.charges.charge(address, bit, self.bytes[address]);
                            value |= u8::from(sensed_bit(address, bit, q, mode)) << bit;
                        }
                        self.sensed[lane] = value;
                    }
                }
                self.verify_pending = None;
            }
        }
    }
    fn finish_pulse(
        &self,
        previous: Option<Pulse>,
        now: Time,
        complete: bool,
        out: &mut dyn Output,
    ) {
        let Some((_, start, end, _)) = previous.filter(|old| Some(*old) != self.pulse()) else {
            return;
        };
        if start == end {
            return;
        }
        let domain = NvDomain::InternalFlash;
        let address = start as u16;
        let length = (end - start) as u16;
        let _ = out.event(if complete {
            Event::NvCommit {
                at: now,
                domain,
                address,
                length,
            }
        } else {
            Event::NvInterrupted {
                at: now,
                domain,
                address,
                length,
            }
        });
    }
    pub(crate) fn protect(&mut self, now: Time, out: &mut dyn Output) -> Result<(), Error> {
        let previous = self.pulse();
        if let Some((program, _, _, _)) = previous {
            self.settle(now, out);
            self.error = true;
            self.finish_pulse(previous, now, false, out);
            self.recovery = self.recovery.max(after(now, if program { 5 } else { 10 })?);
        }
        Ok(())
    }
    pub(crate) fn normal(&self, now: Time) -> bool {
        self.control & 0x3f == 0
            && self.supply != Supply::Stopped
            && now >= self.ready.max(self.recovery)
    }
    pub(super) fn normal_read(&self, now: Time) -> Option<NormalRead> {
        self.normal(now).then_some(NormalRead(()))
    }
    #[inline]
    pub(super) fn read_normal(&self, _: NormalRead, address: u16, width: Width) -> u16 {
        let i = usize::from(address);
        if width == Width::Word {
            u16::from_be_bytes([self.bytes[i], self.bytes[i + 1]])
        } else {
            u16::from(self.bytes[i])
        }
    }
    pub fn read8(&mut self, address: u16, now: Time, out: &mut dyn Output) -> Result<u8, Error> {
        if let Some(normal) = self.normal_read(now) {
            return Ok(self.read_normal(normal, address, Width::Byte) as u8);
        }
        let pulse = self.pulse().is_some();
        if pulse {
            self.protect(now, out)?;
        } else {
            self.settle(now, out);
        }
        if !pulse
            && self.supply != Supply::Stopped
            && now >= self.ready
            && matches!(self.control & 15, PV | EV)
        {
            return Ok(self.sensed[usize::from(address & 3)]);
        }
        Ok(0xff)
    }
    pub fn read16(&mut self, address: u16, now: Time, out: &mut dyn Output) -> Result<u16, Error> {
        if let Some(normal) = self.normal_read(now) {
            return Ok(self.read_normal(normal, address, Width::Word));
        }
        Ok(u16::from_be_bytes([
            self.read8(address, now, out)?,
            self.read8(address + 1, now, out)?,
        ]))
    }
    pub fn write8(
        &mut self,
        address: u16,
        value: u8,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        self.settle(now, out);
        if self.control & SWE == 0
            || now < self.swe_ready
            || self.supply != Supply::Normal
            || !self.oscillator
        {
            return Ok(());
        }
        match self.control & 15 {
            0 if now >= self.recovery => {
                self.latch[usize::from(address) % PAGE] = value;
                self.page = address & !(PAGE as u16 - 1);
            }
            PV | EV => {
                let ready = after(now, 2)?
                    .max(self.verify_ready)
                    .max(self.recovery)
                    .max(self.ready);
                self.verify_pending = Some((address & !3, ready));
            }
            _ => {}
        }
        Ok(())
    }
    fn initialize_control(&mut self) {
        self.control = 0;
        self.error = false;
        self.erase = 0;
        self.verify_pending = None;
    }
    pub fn reset(&mut self, now: Time, out: &mut dyn Output) {
        let previous = self.pulse();
        self.settle(now, out);
        self.initialize_control();
        self.finish_pulse(previous, now, false, out);
        self.enabled = false;
        self.power_down_disabled = false;
        self.latch = [0xff; PAGE];
        self.page = 0;
        self.sensed = [0xff; 4];
        self.recovery = now;
    }
    pub fn power_off(&mut self, now: Time, out: &mut dyn Output) {
        self.reset(now, out);
        self.supply = Supply::Stopped;
        self.oscillator = false;
        self.module = false;
    }
    pub fn environment(
        &mut self,
        mode: Mode,
        module: bool,
        oscillator: bool,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        let supply = if !module || matches!(mode, Mode::Subsleep | Mode::Watch | Mode::Standby) {
            Supply::Stopped
        } else if mode == Mode::Subactive && !self.power_down_disabled {
            Supply::Reduced
        } else {
            Supply::Normal
        };
        if self.supply == supply
            && self.oscillator == oscillator
            && self.mode == mode
            && self.module == module
        {
            return Ok(());
        }
        let previous = self.pulse();
        self.settle(now, out);
        if (!module && self.module)
            || (mode != self.mode && !matches!(mode, Mode::Active | Mode::Sleep))
        {
            self.initialize_control();
        }
        if supply != self.supply && self.supply != Supply::Normal && supply != Supply::Stopped {
            self.ready = after(now, 20)?;
        }
        if oscillator && !self.oscillator {
            self.swe_ready = after(now, 1)?;
            self.program_ready = after(now, 50)?;
            self.erase_ready = after(now, 100)?;
        }
        self.supply = supply;
        self.oscillator = oscillator;
        self.mode = mode;
        self.module = module;
        self.finish_pulse(previous, now, false, out);
        Ok(())
    }
    /// Physical nonvolatile image, including unfinished exposure, without a
    /// guest read, error-protection event, or completion of the active pulse.
    pub fn image(&self, now: Time) -> Box<[u8; FLASH_SIZE]> {
        let mut view = self.clone();
        view.settle(now, &mut ());
        view.bytes
    }
    pub fn peek(&self, address: u16, now: Time) -> u8 {
        if self.control & (P | E | PV | EV) == 0 {
            return self.bytes[usize::from(address)];
        }
        let mut view = self.clone();
        view.settle(now, &mut ());
        view.bytes[usize::from(address)]
    }
    pub(crate) fn settled_byte(&self, address: u16) -> u8 {
        self.bytes[usize::from(address)]
    }
}

// Only touched physical pages are explicit. Allocation is bounded before any
// file-supplied count can reserve memory; the index is rebuilt, never trusted.
impl borsh::BorshSerialize for Charges {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        self.pages.serialize(writer)
    }
}
impl borsh::BorshDeserialize for Charges {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let count = u32::deserialize_reader(reader)?;
        if count > PAGES as u32 {
            return Err(std::io::Error::other("too many flash pages"));
        }
        let mut charges = Self::new();
        for i in 0..count {
            let page = ChargePage::deserialize_reader(reader)?;
            let address = usize::from(page.address);
            if address >= FLASH_SIZE
                || address % PAGE != 0
                || charges.indices[address / PAGE] != u16::MAX
                || page.cells.iter().any(|q| *q > FULL)
            {
                return Err(std::io::Error::other("invalid flash charge page"));
            }
            charges.indices[address / PAGE] = i as u16;
            charges.pages.push(page);
        }
        Ok(charges)
    }
}
impl Flash {
    pub(crate) fn validate(&self, now: Time) -> Result<(), Error> {
        crate::state::require(
            self.control & !0x7f == 0
                && self.erase & !0x3f == 0
                && usize::from(self.page) < FLASH_SIZE
                && self.page & 127 == 0
                && self.pulse_since <= now
                && self.verify_pending.is_none_or(|(address, _)| {
                    usize::from(address) < FLASH_SIZE && address & 3 == 0
                }),
            "invalid flash progress",
        )
    }
}
