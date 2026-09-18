//! Functional contract of the unavailable manufacturer boot ROM (§6.3).
//! All accesses use the ordinary MCU bus, SCI shifter and flash pulse owner;
//! uploaded code enters the sole H8 interpreter. Private ROM instruction timing
//! and scratch registers are not known. See h8-boot-mode-implementation.md.
#[cfg(test)]
mod tests;

use crate::{
    cpu::{Action, Width},
    error::Error,
    mcu::{clocks::Tap, Mcu},
    state::require,
    time::{Time, TimeError},
};
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Sequence {
    Setup = 0,
    Baud = 1,
    FlashBegin = 2,
    Pulse = 3,
    VerifyEnd = 4,
    FlashEnd = 5,
    Handoff = 6,
}
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Receive {
    Confirm = 0,
    LengthHigh = 1,
    LengthLow = 2,
    Payload = 3,
}
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum AfterSend {
    Receive(Receive) = 0,
    FinalAck = 1,
    Drain = 2,
    Halt = 3,
}
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Stage {
    Sequence(Sequence, u8) = 0,
    Measure = 1,
    Receive(Receive) = 2,
    ReceiveStatus(Receive) = 3,
    ReceiveData(Receive) = 4,
    BlankRead = 5,
    VerifyDummy = 6,
    VerifyWait = 7,
    VerifyHigh = 8,
    VerifyLow = 9,
    Store = 10,
    Send(AfterSend) = 11,
    SendStatus(AfterSend) = 12,
    SendByte(AfterSend) = 13,
    Drain = 14,
    Done = 15,
    Halted = 16,
}
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub(super) struct Boot {
    stage: Stage,
    low_start: Option<u64>,
    saw_high: bool,
    brr: u8,
    length: u16,
    cursor: u16,
    address: u16,
    block: u8,
    attempt: u8,
    verify_ok: bool,
    failed: bool,
    byte: u8,
}
const BLOCKS: [u16; 7] = [0, 0x400, 0x800, 0xc00, 0x1000, 0x8000, 0xc000];
fn read(address: u16, width: Width) -> Action {
    Action::Read {
        address,
        width,
        fetch: false,
    }
}
fn write(address: u16, value: u8) -> Action {
    Action::Write {
        address,
        width: Width::Byte,
        value: u16::from(value),
        mov_byte: true,
    }
}
fn wait(micros: u32, hz: u64) -> Result<Action, Error> {
    let states = (u128::from(hz) * u128::from(micros))
        .div_ceil(1_000_000)
        .max(1);
    Ok(Action::Idle(
        u32::try_from(states).map_err(|_| TimeError::Overflow)?,
    ))
}
impl Boot {
    pub(super) fn new(test_high: bool) -> Self {
        Self {
            stage: if test_high {
                Stage::Halted
            } else {
                Stage::Sequence(Sequence::Setup, 0)
            },
            low_start: None,
            saw_high: false,
            brr: 255,
            length: 0,
            cursor: 0,
            address: 0,
            block: 0,
            attempt: 0,
            verify_ok: true,
            failed: false,
            byte: 0,
        }
    }
    fn sequence(&self, sequence: Sequence, i: u8, hz: u64) -> Result<Option<Action>, Error> {
        use Sequence::*;
        let stop_wdt = |i| match i {
            0 => 0x1e,
            1 => 0xa2,
            _ => 0x8e,
        };
        Ok(Some(match (sequence, i) {
            (Setup, 0..=2) => write(0xffb1, stop_wdt(i)),
            (Setup, 3) => write(0xfffa, 0x43),
            (Setup, 4) => write(0xff9a, 0),
            (Setup, 5) => write(0xff91, 0xc0),
            (Setup, 6) => write(0xff98, 0),
            (Setup, 7) => write(0xffa6, 0),
            (Setup, 8) => write(0xffa7, 0),
            (Setup, 9) => write(0xffd6, 4),
            (Setup, 10) => write(0xffe6, 4),
            // Register bus cycles above consume 25 of the nominal 100 states.
            (Setup, 11) => Action::Idle(75),
            (Baud, 0) => write(0xff99, self.brr),
            (Baud, 1) => Action::Idle(32 * (u32::from(self.brr) + 1)),
            (Baud, 2) => write(0xff91, 0xd0),
            (Baud, 3) => write(0xff9a, 0x30),
            (FlashBegin, 0) => write(0xf02b, 0x80),
            (FlashBegin, 1) => write(0xf020, 0x40),
            (FlashBegin, 2) => wait(1, hz)?,
            (FlashBegin, 3) => write(0xffb0, 0xff), // WDT phi/8192
            (Pulse, 0) => write(0xf023, 1 << self.block),
            (Pulse, 1) => write(0xffb1, 0x5e), // enable control and counter writes
            (Pulse, 2) => write(0xffb3, {
                let ticks = (u128::from(hz) * 198).div_ceil(8192 * 10_000).clamp(1, 256);
                (256 - ticks) as u8
            }),
            (Pulse, 3) => write(0xffb1, 0x96), // TME=1, retain TCWE
            (Pulse, 4) => write(0xf020, 0x60),
            (Pulse, 5) => wait(100, hz)?,
            (Pulse, 6) => write(0xf020, 0x62),
            (Pulse, 7) => wait(10_000, hz)?,
            (Pulse, 8) => write(0xf020, 0x60),
            (Pulse, 9) => wait(10, hz)?,
            (Pulse, 10) => write(0xf020, 0x40),
            (Pulse, 11) => wait(10, hz)?,
            (Pulse, 12..=14) => write(0xffb1, stop_wdt(i - 12)),
            (Pulse, 15) => write(0xf020, 0x48),
            (Pulse, 16) => wait(20, hz)?,
            (VerifyEnd, 0) => write(0xf020, 0x40),
            (VerifyEnd, 1) => wait(4, hz)?,
            (FlashEnd, 0) => write(0xf020, 0),
            (FlashEnd, 1) => wait(100, hz)?,
            (FlashEnd, 2) => write(0xf02b, 0),
            (Handoff, 0) => write(0xff9a, 0),
            (Handoff, 1) => write(0xffd6, 4),
            (Handoff, 2) => write(0xffe6, 4),
            (Handoff, 3) => write(0xff91, 0xc0),
            _ => return Ok(None),
        }))
    }
    pub(super) fn action(&self, hz: u64) -> Result<Option<Action>, Error> {
        Ok(match self.stage {
            Stage::Sequence(s, i) => self.sequence(s, i, hz)?,
            Stage::ReceiveStatus(_) | Stage::SendStatus(_) => Some(read(0xff9c, Width::Byte)),
            Stage::ReceiveData(_) => Some(read(0xff9d, Width::Byte)),
            Stage::BlankRead | Stage::VerifyHigh => Some(read(self.address, Width::Word)),
            Stage::VerifyLow => Some(read(self.address + 2, Width::Word)),
            Stage::VerifyDummy => Some(write(self.address, 255)),
            Stage::VerifyWait => Some(wait(2, hz)?),
            Stage::Store => Some(write(0xfb80 + self.cursor, self.byte)),
            Stage::SendByte(_) => Some(write(0xff9b, self.byte)),
            _ => None,
        })
    }
    pub(super) fn next(&mut self, now: Time, mcu: &Mcu) -> Result<Option<Action>, Error> {
        if self.stage == Stage::VerifyDummy && mcu.flash.register(0xf021) & 0x80 != 0 {
            self.failed = true;
            self.stage = Stage::Sequence(Sequence::VerifyEnd, 0);
        }
        match self.stage {
            Stage::Measure => {
                let high = mcu.gpio.sci_inputs().1;
                let tick = mcu.clocks.ticks(now, Tap::system(1));
                if high {
                    self.saw_high = true;
                    if let Some(start) = self.low_start.take() {
                        let q = tick.saturating_sub(start).saturating_add(144) / 288;
                        if (1..=256).contains(&q) {
                            self.brr = (q - 1) as u8;
                            self.stage = Stage::Sequence(Sequence::Baud, 0);
                        }
                    }
                } else if self.saw_high && self.low_start.is_none() {
                    self.low_start = Some(tick);
                }
            }
            Stage::Receive(r) if mcu.sci.peek(0xff9c) & 0x78 != 0 => {
                self.stage = Stage::ReceiveStatus(r)
            }
            Stage::Send(a) if mcu.sci.peek(0xff9c) & 0x80 != 0 => self.stage = Stage::SendStatus(a),
            Stage::Drain if mcu.sci.transmit_idle() => {
                self.stage = Stage::Sequence(Sequence::Handoff, 0)
            }
            _ => {}
        }
        self.action(mcu.clocks.frequencies.main_hz)
    }
    fn send(&mut self, value: u8, after: AfterSend) {
        self.byte = value;
        self.stage = Stage::Send(after);
    }
    pub(super) fn complete(&mut self, value: u16, hz: u64) -> Result<(), Error> {
        use Sequence::*;
        match self.stage {
            Stage::Sequence(s, i) => {
                if self.sequence(s, i + 1, hz)?.is_some() {
                    self.stage = Stage::Sequence(s, i + 1);
                } else {
                    match s {
                        Setup => self.stage = Stage::Measure,
                        Baud => self.send(0, AfterSend::Receive(Receive::Confirm)),
                        FlashBegin => {
                            self.attempt = 1;
                            self.stage = Stage::Sequence(Pulse, 0);
                        }
                        Pulse => {
                            self.verify_ok = true;
                            self.address = BLOCKS[usize::from(self.block)];
                            self.stage = Stage::VerifyDummy;
                        }
                        VerifyEnd => {
                            if self.verify_ok {
                                self.block += 1;
                                self.attempt = 0;
                            }
                            if self.block == 6 || self.attempt == 100 || self.failed {
                                self.failed |= self.attempt == 100 && !self.verify_ok;
                                self.stage = Stage::Sequence(FlashEnd, 0);
                            } else {
                                self.attempt += 1;
                                self.stage = Stage::Sequence(Pulse, 0);
                            }
                        }
                        FlashEnd => self.send(
                            if self.failed { 0xff } else { 0xaa },
                            if self.failed {
                                AfterSend::Halt
                            } else {
                                AfterSend::Receive(Receive::LengthHigh)
                            },
                        ),
                        Handoff => self.stage = Stage::Done,
                    }
                }
            }
            Stage::ReceiveStatus(r) => {
                self.stage = if value & 0x38 != 0 {
                    Stage::Halted
                } else if value & 0x40 != 0 {
                    Stage::ReceiveData(r)
                } else {
                    Stage::Receive(r)
                }
            }
            Stage::ReceiveData(r) => match r {
                Receive::Confirm => {
                    self.address = 0;
                    self.block = 0;
                    self.attempt = 0;
                    self.stage = if value == 0x55 {
                        Stage::BlankRead
                    } else {
                        Stage::Receive(r)
                    };
                }
                Receive::LengthHigh => {
                    self.length = value << 8;
                    self.send(value as u8, AfterSend::Receive(Receive::LengthLow));
                }
                Receive::LengthLow => {
                    self.length |= value;
                    self.cursor = 0;
                    let after = if (1..=1024).contains(&self.length) {
                        AfterSend::Receive(Receive::Payload)
                    } else {
                        AfterSend::Halt
                    };
                    self.send(value as u8, after);
                }
                Receive::Payload => {
                    self.byte = value as u8;
                    self.stage = Stage::Store;
                }
            },
            Stage::BlankRead => {
                if value != 0xffff {
                    self.address = 0;
                    self.stage = Stage::Sequence(FlashBegin, 0);
                } else {
                    self.address += 2;
                    if self.address == 0xc000 {
                        self.send(0xaa, AfterSend::Receive(Receive::LengthHigh));
                    }
                }
            }
            Stage::VerifyDummy => self.stage = Stage::VerifyWait,
            Stage::VerifyWait => self.stage = Stage::VerifyHigh,
            Stage::VerifyHigh => {
                self.verify_ok &= value == 0xffff;
                self.stage = Stage::VerifyLow;
            }
            Stage::VerifyLow => {
                self.verify_ok &= value == 0xffff;
                self.address += 4;
                self.stage =
                    if !self.verify_ok || self.address == BLOCKS[usize::from(self.block) + 1] {
                        Stage::Sequence(VerifyEnd, 0)
                    } else {
                        Stage::VerifyDummy
                    };
            }
            Stage::Store => {
                self.cursor += 1;
                self.stage = Stage::Send(if self.cursor == self.length {
                    AfterSend::FinalAck
                } else {
                    AfterSend::Receive(Receive::Payload)
                });
            }
            Stage::SendStatus(a) => {
                self.stage = if value & 0x80 != 0 {
                    Stage::SendByte(a)
                } else {
                    Stage::Send(a)
                }
            }
            Stage::SendByte(a) => match a {
                AfterSend::Receive(r) => self.stage = Stage::Receive(r),
                AfterSend::FinalAck => self.send(0xaa, AfterSend::Drain),
                AfterSend::Drain => self.stage = Stage::Drain,
                AfterSend::Halt => self.stage = Stage::Halted,
            },
            _ => return Err(Error::Internal("boot completion without an issued access")),
        }
        Ok(())
    }
    pub(super) fn done(&self) -> bool {
        self.stage == Stage::Done
    }
    pub(super) fn validate(&self, now: Time, mcu: &Mcu) -> Result<(), Error> {
        require(
            self.block <= 6
                && self.attempt <= 100
                && self.cursor <= 1024
                && self.address <= 0xc000
                && self.address & 1 == 0
                && self
                    .low_start
                    .is_none_or(|n| n <= mcu.clocks.ticks(now, Tap::system(1))),
            "invalid boot-service progress",
        )?;
        if let Stage::Sequence(s, i) = self.stage {
            require(
                i < 17
                    && self
                        .sequence(s, i, mcu.clocks.frequencies.main_hz)?
                        .is_some(),
                "invalid boot-service sequence",
            )?;
        }
        if matches!(self.stage, Stage::Sequence(Sequence::FlashBegin, _)) {
            require(
                self.block == 0 && self.attempt == 0,
                "invalid boot erase admission",
            )?;
        }
        if matches!(
            self.stage,
            Stage::Sequence(Sequence::Pulse | Sequence::VerifyEnd, _)
                | Stage::VerifyDummy
                | Stage::VerifyWait
                | Stage::VerifyHigh
                | Stage::VerifyLow
        ) {
            require(
                self.block < 6 && self.address & 3 == 0,
                "invalid boot erase block",
            )?;
        }
        if matches!(
            self.stage,
            Stage::BlankRead
                | Stage::VerifyDummy
                | Stage::VerifyWait
                | Stage::VerifyHigh
                | Stage::VerifyLow
        ) {
            require(self.address < 0xc000, "invalid boot flash address")?;
        }
        if matches!(
            self.stage,
            Stage::VerifyDummy | Stage::VerifyWait | Stage::VerifyHigh | Stage::VerifyLow
        ) {
            require(
                (BLOCKS[usize::from(self.block)]..BLOCKS[usize::from(self.block) + 1])
                    .contains(&self.address),
                "boot verify address outside selected block",
            )?;
        }
        if matches!(
            self.stage,
            Stage::Store
                | Stage::Receive(Receive::Payload)
                | Stage::ReceiveStatus(Receive::Payload)
                | Stage::ReceiveData(Receive::Payload)
                | Stage::Send(AfterSend::Receive(Receive::Payload))
                | Stage::SendStatus(AfterSend::Receive(Receive::Payload))
                | Stage::SendByte(AfterSend::Receive(Receive::Payload))
        ) {
            require(
                (1..=1024).contains(&self.length) && self.cursor < self.length,
                "invalid boot upload aperture",
            )?;
        }
        Ok(())
    }
}
