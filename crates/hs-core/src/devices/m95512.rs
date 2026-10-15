//! ST M95512 main-array protocol driven through its pins. Serial command state
//! and an ongoing nonvolatile operation have independent lifetimes.
use super::nv::WriteCycle;
use crate::{
    error::Error,
    signals::{Drive, Event, NvDomain, Output},
    time::{Duration, Time},
};
pub const EEPROM_SIZE: usize = 65536;
pub const PAGE_SIZE: usize = 128;
const PERSISTENT_MASK: u8 = 0x8c;

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Command {
    Opcode = 0,
    Enable = 1,
    Disable = 2,
    ReadStatus = 3,
    WriteStatus = 4,
    Address { write: bool, high: Option<u8> } = 5,
    Read = 6,
    Write = 7,
    Ignore = 8,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Programming {
    None = 0,
    Page { cycle: WriteCycle, base: u16 } = 1,
    Status { cycle: WriteCycle, value: u8 } = 2,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct M95512 {
    #[borsh(deserialize_with = "crate::state::read_bytes")]
    array: Box<[u8; EEPROM_SIZE]>,
    status: u8,
    wel: bool,
    selected: bool,
    select_high_seen: bool,
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
            select_high_seen: false,
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
    /// Project persistent cells without advancing protocol state or committing
    /// a pending operation. The caller owns this ordinary EEPROM-save image.
    pub fn bytes(&self, now: Time) -> [u8; EEPROM_SIZE] {
        let mut bytes = *self.array;
        if let Programming::Page { cycle, base } = self.programming {
            for index in 0..PAGE_SIZE {
                if self.written[index / 64] & (1u64 << (index % 64)) != 0 {
                    let address = base + index as u16;
                    bytes[usize::from(address)] = cycle.byte(
                        self.array[usize::from(address)],
                        self.page[index],
                        u32::from(address),
                        now,
                    );
                }
            }
        }
        bytes
    }
    pub fn persistent_status(&self, now: Time) -> u8 {
        match self.programming {
            Programming::Status { cycle, value } => {
                cycle.byte(self.status, value, 0x10000, now) & PERSISTENT_MASK
            }
            _ => self.status,
        }
    }
    pub fn busy(&self) -> bool {
        self.programming != Programming::None
    }
    pub(crate) fn write_bytes(&mut self, address: u16, bytes: &[u8]) -> Result<(), Error> {
        if self.busy() {
            return Err(Error::BadInput("cannot edit EEPROM during programming"));
        }
        let start = usize::from(address);
        let end = start
            .checked_add(bytes.len())
            .ok_or(Error::BadInput("edit extends beyond EEPROM"))?;
        self.array
            .get_mut(start..end)
            .ok_or(Error::BadInput("edit extends beyond EEPROM"))?
            .copy_from_slice(bytes);
        Ok(())
    }
    pub fn status(&self) -> u8 {
        self.status | (u8::from(self.wel) * 2) | u8::from(self.busy())
    }
    /// Serial clock edges until the byte completes, from the clock's level.
    pub(crate) fn serial_effect_edges(&self, clock_high: bool) -> u8 {
        if self.selected {
            (8 - self.rx_bits) * 2 - u8::from(!clock_high)
        } else {
            16
        }
    }
    pub fn deadline(&self) -> Option<Time> {
        match self.programming {
            Programming::None => None,
            Programming::Page { cycle, .. } | Programming::Status { cycle, .. } => {
                Some(cycle.deadline)
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
        self.select_effect(selected, now).map(|_| ())
    }
    /// Releasing CS can start a self-timed write. Shifting and an unchanged
    /// selection leave the programming appointment untouched.
    pub(crate) fn select_effect(&mut self, selected: bool, now: Time) -> Result<bool, Error> {
        if !selected {
            self.select_high_seen = true;
        } else if !self.select_high_seen {
            return Ok(false);
        }
        if self.selected == selected {
            return Ok(false);
        }
        let was_busy = self.busy();
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
                            self.programming = Programming::Page {
                                cycle: WriteCycle::start(now, self.write_time)?,
                                base,
                            };
                        }
                    }
                    Command::WriteStatus
                        if self.wel
                            && self.pending_status.is_some()
                            && !(self.write_protect_low && self.status & 0x80 != 0) =>
                    {
                        let value = self.pending_status.unwrap_or(0) & PERSISTENT_MASK;
                        self.programming = Programming::Status {
                            cycle: WriteCycle::start(now, self.write_time)?,
                            value,
                        };
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
        Ok(!was_busy && self.busy())
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
    /// Compose falling/rising pairs. Split at receive and transmit byte
    /// boundaries so every command still enters the same byte parser.
    pub(crate) fn exchange_bits(&mut self, input: u8, count: u8, floating: bool) -> u8 {
        if !self.selected {
            self.driven = Drive::Floating;
            return if floating { 0xff >> (8 - count) } else { 0 };
        }
        let mut remaining = count;
        let mut output = 0u8;
        while remaining != 0 {
            let transmitting = self.tx_bit < 8;
            let n =
                remaining
                    .min(8 - self.rx_bits)
                    .min(if transmitting { 8 - self.tx_bit } else { 8 });
            let mask = 0xff >> (8 - n);
            let answer = if transmitting {
                self.tx_bit += n;
                let bits = (self.tx >> (8 - self.tx_bit)) & mask;
                self.driven = if bits & 1 == 0 {
                    Drive::Low
                } else {
                    Drive::High
                };
                bits
            } else {
                self.driven = Drive::Floating;
                if floating {
                    mask
                } else {
                    0
                }
            };
            output = ((u16::from(output) << n) | u16::from(answer)) as u8;
            self.rx =
                ((u16::from(self.rx) << n) | u16::from((input >> (remaining - n)) & mask)) as u8;
            self.rx_bits += n;
            remaining -= n;
            if self.rx_bits == 8 {
                let byte = self.rx;
                self.rx = 0;
                self.rx_bits = 0;
                self.byte(byte);
            }
        }
        output
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
                if self.pending_status.is_some() {
                    self.command = Command::Ignore;
                } else {
                    self.pending_status = Some(byte);
                }
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
        self.settle_cells(now, output, true);
        Ok(())
    }
    fn settle_cells(&mut self, now: Time, output: &mut dyn Output, complete: bool) {
        let (domain, address, length) = match self.programming {
            Programming::Page { cycle, base } => {
                for index in 0..PAGE_SIZE {
                    if self.written[index / 64] & (1u64 << (index % 64)) != 0 {
                        let address = base + index as u16;
                        let value = cycle.byte(
                            self.array[usize::from(address)],
                            self.page[index],
                            u32::from(address),
                            now,
                        );
                        self.array[usize::from(address)] = value;
                        let _ = output.event(Event::NvByte {
                            at: now,
                            domain: NvDomain::EepromArray,
                            address,
                            value,
                        });
                    }
                }
                let selected = u128::from(self.written[0]) | (u128::from(self.written[1]) << 64);
                let first = selected.trailing_zeros() as u16;
                let end = PAGE_SIZE as u16 - selected.leading_zeros() as u16;
                (NvDomain::EepromArray, base + first, end - first)
            }
            Programming::Status { .. } => {
                let value = self.persistent_status(now);
                self.status = value;
                let _ = output.event(Event::NvByte {
                    at: now,
                    domain: NvDomain::EepromStatus,
                    address: 0,
                    value,
                });
                (NvDomain::EepromStatus, 0, 1)
            }
            Programming::None => return,
        };
        let _ = output.event(if complete {
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
        self.programming = Programming::None;
        self.wel = false;
        self.written = [0; 2];
    }
    pub fn power_off(&mut self, now: Time, output: &mut dyn Output) {
        self.settle_cells(now, output, false);
        self.wel = false;
        self.selected = false;
        self.select_high_seen = false;
        self.command = Command::Opcode;
        self.rx = 0;
        self.rx_bits = 0;
        self.tx_bit = 8;
        self.driven = Drive::Floating;
        self.pending_status = None;
        self.data_count = 0;
        self.written = [0; 2];
    }
}

impl M95512 {
    pub(crate) fn validate(&self, now: Time) -> Result<(), Error> {
        use crate::state::require;
        require(
            self.status & !PERSISTENT_MASK == 0
                && self.rx_bits < 8
                && self.tx_bit <= 8
                && self.write_time != Duration::ZERO,
            "invalid EEPROM state",
        )?;
        match self.programming {
            Programming::None => Ok(()),
            Programming::Page { cycle, base } => {
                require(base & 127 == 0, "unaligned EEPROM write page")?;
                require(self.written != [0; 2], "empty EEPROM write operation")?;
                cycle.validate(now)
            }
            Programming::Status { cycle, value } => {
                require(value & !PERSISTENT_MASK == 0, "invalid EEPROM status write")?;
                cycle.validate(now)
            }
        }
    }
}
