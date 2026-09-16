//! ST M95512 main-array protocol. Pin-level input and output are canonical;
//! no MCU-only byte shortcut exists. Serial command state and an ongoing
//! nonvolatile operation have independent lifetimes.
use crate::{
    error::Error,
    signals::{Drive, Event, NvDomain, Output},
    time::{Duration, Time},
};
pub const EEPROM_SIZE: usize = 65536;
pub const PAGE_SIZE: usize = 128;
const PERSISTENT_MASK: u8 = 0x8c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Opcode,
    Enable,
    Disable,
    ReadStatus,
    WriteStatus,
    Address { write: bool, high: Option<u8> },
    Read,
    Write,
    Ignore,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Programming {
    None,
    Page { deadline: Time, base: u16 },
    Status { deadline: Time, value: u8 },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M95512 {
    array: Box<[u8; EEPROM_SIZE]>,
    status: u8,
    wel: bool,
    selected: bool,
    command: Command,
    rx: u8,
    rx_bits: u8,
    tx: u8,
    tx_bit: u8,
    driven: Drive,
    address: u16,
    page: [u8; PAGE_SIZE],
    written: [u64; 2],
    data_count: u16,
    pending_status: Option<u8>,
    programming: Programming,
    write_time: Duration,
    write_protect_low: bool,
}
impl M95512 {
    pub fn new(bytes: &[u8], status: u8) -> Result<Self, Error> {
        if bytes.len() != EEPROM_SIZE {
            return Err(Error::ImageSize {
                name: "EEPROM",
                expected: EEPROM_SIZE,
                actual: bytes.len(),
            });
        }
        if status & !PERSISTENT_MASK != 0 {
            return Err(Error::PersistentStatus(status));
        }
        let array: Box<[u8; EEPROM_SIZE]> = bytes
            .to_vec()
            .into_boxed_slice()
            .try_into()
            .map_err(|_| Error::Internal("EEPROM allocation shape"))?;
        Ok(Self {
            array,
            status,
            wel: false,
            selected: false,
            command: Command::Opcode,
            rx: 0,
            rx_bits: 0,
            tx: 0xff,
            tx_bit: 8,
            driven: Drive::Floating,
            address: 0,
            page: [0xff; PAGE_SIZE],
            written: [0; 2],
            data_count: 0,
            pending_status: None,
            programming: Programming::None,
            write_time: Duration::from_millis(5),
            write_protect_low: false,
        })
    }
    pub fn bytes(&self) -> &[u8; EEPROM_SIZE] {
        &self.array
    }
    pub fn persistent_status(&self) -> u8 {
        self.status
    }
    pub fn busy(&self) -> bool {
        self.programming != Programming::None
    }
    pub fn status(&self) -> u8 {
        self.status | (u8::from(self.wel) * 2) | u8::from(self.busy())
    }
    pub fn deadline(&self) -> Option<Time> {
        match self.programming {
            Programming::None => None,
            Programming::Page { deadline, .. } | Programming::Status { deadline, .. } => {
                Some(deadline)
            }
        }
    }
    pub fn set_write_duration(&mut self, duration: Duration) -> Result<(), Error> {
        if self.busy() || duration == Duration::ZERO {
            return Err(Error::BadInput(
                "cannot alter EEPROM timing while busy or select zero duration",
            ));
        }
        self.write_time = duration;
        Ok(())
    }
    /// An advanced fixture control. The normal Pokewalker board fixes WP high.
    pub fn set_write_protect(&mut self, low: bool) {
        self.write_protect_low = low;
    }
    pub fn set_selected(&mut self, selected: bool, now: Time) -> Result<(), Error> {
        if self.selected == selected {
            return Ok(());
        }
        if !selected {
            let complete_byte = self.rx_bits == 0;
            if !self.busy() && complete_byte {
                match self.command {
                    Command::Enable => self.wel = true,
                    Command::Disable => self.wel = false,
                    Command::Write if self.wel && self.data_count != 0 => {
                        let base = self.address & !127;
                        let protected_start = match (self.status >> 2) & 3 {
                            0 => 65536,
                            1 => 49152,
                            2 => 32768,
                            _ => 0,
                        };
                        if usize::from(base) < protected_start {
                            let deadline = now
                                .checked_add(self.write_time)
                                .ok_or(crate::time::TimeError::Overflow)?;
                            self.programming = Programming::Page { deadline, base };
                            self.wel = false;
                        }
                    }
                    Command::WriteStatus
                        if self.wel
                            && self.pending_status.is_some()
                            && !(self.write_protect_low && self.status & 0x80 != 0) =>
                    {
                        let value = self.pending_status.unwrap_or(0) & PERSISTENT_MASK;
                        let deadline = now
                            .checked_add(self.write_time)
                            .ok_or(crate::time::TimeError::Overflow)?;
                        self.programming = Programming::Status { deadline, value };
                        self.wel = false;
                    }
                    _ => {}
                }
            }
        }
        self.selected = selected;
        self.command = Command::Opcode;
        self.rx = 0;
        self.rx_bits = 0;
        self.tx = 0xff;
        self.tx_bit = 8;
        self.driven = Drive::Floating;
        self.pending_status = None;
        self.data_count = 0;
        // The programming payload cannot be overwritten by a new selected
        // command while the self-timed write is active.
        if !self.busy() {
            self.written = [0; 2];
        }
        Ok(())
    }
    pub fn rising(&mut self, mosi: bool) {
        if !self.selected {
            return;
        }
        self.rx = (self.rx << 1) | u8::from(mosi);
        self.rx_bits += 1;
        if self.rx_bits == 8 {
            let byte = self.rx;
            self.rx = 0;
            self.rx_bits = 0;
            self.byte(byte);
        }
    }
    pub fn falling(&mut self) -> Drive {
        if !self.selected || self.tx_bit >= 8 {
            self.driven = Drive::Floating;
        } else {
            self.driven = if self.tx & (0x80 >> self.tx_bit) == 0 {
                Drive::Low
            } else {
                Drive::High
            };
            self.tx_bit += 1;
        }
        self.driven
    }
    pub fn output(&self) -> Drive {
        if self.selected {
            self.driven
        } else {
            Drive::Floating
        }
    }
    fn transmit(&mut self, byte: u8) {
        self.tx = byte;
        self.tx_bit = 0;
    }
    fn byte(&mut self, byte: u8) {
        match self.command {
            Command::Opcode => {
                self.command = if self.busy() && byte != 5 {
                    Command::Ignore
                } else {
                    match byte {
                        6 => Command::Enable,
                        4 => Command::Disable,
                        5 => Command::ReadStatus,
                        1 => Command::WriteStatus,
                        3 => Command::Address {
                            write: false,
                            high: None,
                        },
                        2 => Command::Address {
                            write: true,
                            high: None,
                        },
                        _ => Command::Ignore,
                    }
                };
                if self.command == Command::ReadStatus {
                    self.transmit(self.status());
                }
            }
            Command::Enable | Command::Disable => self.command = Command::Ignore,
            Command::ReadStatus => self.transmit(self.status()),
            Command::WriteStatus => {
                self.pending_status = Some(byte);
            }
            Command::Address { write, high: None } => {
                self.command = Command::Address {
                    write,
                    high: Some(byte),
                }
            }
            Command::Address {
                write,
                high: Some(high),
            } => {
                self.address = u16::from(high) << 8 | u16::from(byte);
                self.command = if write { Command::Write } else { Command::Read };
                if !write {
                    self.transmit(self.array[usize::from(self.address)]);
                }
            }
            Command::Read => {
                self.address = self.address.wrapping_add(1);
                self.transmit(self.array[usize::from(self.address)]);
            }
            Command::Write => {
                let index = usize::from(self.address & 127);
                self.page[index] = byte;
                self.written[index / 64] |= 1u64 << (index % 64);
                self.address = (self.address & !127) | (self.address.wrapping_add(1) & 127);
                self.data_count = self.data_count.saturating_add(1);
            }
            Command::Ignore => {}
        }
    }
    pub fn complete(&mut self, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        let Some(deadline) = self.deadline() else {
            return Ok(());
        };
        if now != deadline {
            return Err(Error::Internal(
                "EEPROM completion must occur at its scheduled boundary",
            ));
        }
        match self.programming {
            Programming::Page { base, .. } => {
                let mut count = 0u16;
                for index in 0..PAGE_SIZE {
                    if self.written[index / 64] & (1u64 << (index % 64)) != 0 {
                        let address = base + index as u16;
                        let value = self.page[index];
                        self.array[usize::from(address)] = value;
                        output.event(Event::NvByte {
                            at: now,
                            domain: NvDomain::EepromArray,
                            address,
                            value,
                        });
                        count += 1;
                    }
                }
                output.event(Event::NvCommit {
                    at: now,
                    domain: NvDomain::EepromArray,
                    address: base,
                    length: count,
                });
            }
            Programming::Status { value, .. } => {
                self.status = value;
                output.event(Event::NvByte {
                    at: now,
                    domain: NvDomain::EepromStatus,
                    address: 0,
                    value,
                });
                output.event(Event::NvCommit {
                    at: now,
                    domain: NvDomain::EepromStatus,
                    address: 0,
                    length: 1,
                });
            }
            Programming::None => {}
        }
        self.programming = Programming::None;
        self.written = [0; 2];
        Ok(())
    }
    pub fn power_cycle(&mut self) -> Result<(), Error> {
        if self.busy() {
            return Err(Error::Unsupported {
                component: "M95512",
                detail: "interrupted programming contents require hardware characterization",
                address: self.address,
            });
        }
        self.wel = false;
        self.selected = false;
        self.command = Command::Opcode;
        self.rx = 0;
        self.rx_bits = 0;
        self.tx_bit = 8;
        self.driven = Drive::Floating;
        self.pending_status = None;
        self.data_count = 0;
        self.written = [0; 2];
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn xfer(e: &mut M95512, v: u8) -> u8 {
        let mut result = 0;
        for bit in (0..8).rev() {
            let o = e.falling();
            result = (result << 1) | u8::from(o != Drive::Low);
            e.rising(v & (1 << bit) != 0);
        }
        result
    }
    fn command(e: &mut M95512, bytes: &[u8]) {
        e.set_selected(true, Time::ZERO).unwrap();
        for &v in bytes {
            xfer(e, v);
        }
        e.set_selected(false, Time::ZERO).unwrap();
    }
    #[test]
    fn programming_waits_and_wraps_inside_a_page() {
        let mut e = M95512::new(&[0xff; EEPROM_SIZE], 0).unwrap();
        command(&mut e, &[6]);
        command(&mut e, &[2, 0, 0x7e, 0xaa, 0xbb, 0xcc, 0xdd]);
        assert!(e.busy());
        assert_eq!(e.bytes()[0], 0xff);
        e.complete(e.deadline().unwrap(), &mut ()).unwrap();
        assert_eq!(
            [e.bytes()[0x7e], e.bytes()[0x7f], e.bytes()[0], e.bytes()[1]],
            [0xaa, 0xbb, 0xcc, 0xdd]
        );
        assert!(!e.busy());
        assert_eq!(e.status() & 3, 0);
    }
    #[test]
    fn partial_command_and_missing_write_enable_do_not_program() {
        let mut e = M95512::new(&[0xff; EEPROM_SIZE], 0).unwrap();
        command(&mut e, &[2, 0, 0, 0]);
        assert!(!e.busy());
        e.set_selected(true, Time::ZERO).unwrap();
        for _ in 0..7 {
            e.falling();
            e.rising(false);
        }
        e.set_selected(false, Time::ZERO).unwrap();
        assert_eq!(e.status(), 0);
    }
    #[test]
    fn sequential_read_and_status_stream() {
        let mut bytes = vec![0xff; EEPROM_SIZE];
        bytes[65535] = 0x12;
        bytes[0] = 0x34;
        let mut e = M95512::new(&bytes, 0).unwrap();
        e.set_selected(true, Time::ZERO).unwrap();
        for v in [3, 255, 255] {
            xfer(&mut e, v);
        }
        assert_eq!(xfer(&mut e, 0), 0x12);
        assert_eq!(xfer(&mut e, 0), 0x34);
    }
}
