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
    pub fn status(&self) -> u8 {
        self.status | (u8::from(self.wel) * 2) | u8::from(self.busy())
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
        if !selected {
            self.select_high_seen = true;
        } else if !self.select_high_seen {
            return Ok(());
        }
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
                        output.event(Event::NvByte {
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
                output.event(Event::NvByte {
                    at: now,
                    domain: NvDomain::EepromStatus,
                    address: 0,
                    value,
                });
                (NvDomain::EepromStatus, 0, 1)
            }
            Programming::None => return,
        };
        output.event(if complete {
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
        e.set_selected(false, Time::ZERO).unwrap();
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
        assert_eq!(e.bytes(Time::ZERO)[0], 0xff);
        let completed = e.deadline().unwrap();
        e.complete(completed, &mut ()).unwrap();
        assert_eq!(
            [
                e.bytes(completed)[0x7e],
                e.bytes(completed)[0x7f],
                e.bytes(completed)[0],
                e.bytes(completed)[1]
            ],
            [0xaa, 0xbb, 0xcc, 0xdd]
        );
        assert!(!e.busy());
        assert_eq!(e.status() & 3, 0);
    }
    #[test]
    fn persistent_event_ranges_enclose_offset_and_wrapped_writes() {
        for (start, payload, range) in [
            (0x0156u16, &[0xa5, 0x5a][..], (0x0156, 2)),
            (0x017fu16, &[0xa5, 0x5a][..], (0x0100, 128)),
            (0xffffu16, &[0xa5][..], (0xffff, 1)),
        ] {
            for completed in [false, true] {
                let mut e = M95512::new(&[0xff; EEPROM_SIZE], 0).unwrap();
                command(&mut e, &[6]);
                let mut bytes = vec![2, (start >> 8) as u8, start as u8];
                bytes.extend_from_slice(payload);
                command(&mut e, &bytes);
                let at = if completed {
                    e.deadline().unwrap()
                } else {
                    Time::from_micros(3000)
                };
                let mut events = vec![];
                if completed {
                    e.complete(at, &mut events).unwrap();
                } else {
                    e.power_off(at, &mut events);
                }
                let expected = if completed {
                    Event::NvCommit {
                        at,
                        domain: NvDomain::EepromArray,
                        address: range.0,
                        length: range.1,
                    }
                } else {
                    Event::NvInterrupted {
                        at,
                        domain: NvDomain::EepromArray,
                        address: range.0,
                        length: range.1,
                    }
                };
                assert_eq!(events.pop(), Some(expected));
                assert_eq!(events.len(), payload.len());
                for event in events {
                    let Event::NvByte { address, value, .. } = event else {
                        panic!("expected persistent byte before completion");
                    };
                    assert!(
                        (u32::from(range.0)..u32::from(range.0) + u32::from(range.1))
                            .contains(&u32::from(address))
                    );
                    assert_eq!(e.bytes(at)[usize::from(address)], value);
                }
            }
        }
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
        e.set_selected(false, Time::ZERO).unwrap();
        e.set_selected(true, Time::ZERO).unwrap();
        for v in [3, 255, 255] {
            xfer(&mut e, v);
        }
        assert_eq!(xfer(&mut e, 0), 0x12);
        assert_eq!(xfer(&mut e, 0), 0x34);
    }
    #[test]
    fn interrupted_write_has_erase_then_program_progress_in_only_addressed_cells() {
        let mut e = M95512::new(&[0xff; EEPROM_SIZE], 0).unwrap();
        command(&mut e, &[6]);
        command(&mut e, &[2, 0, 0x7f, 0xa5, 0xff]);
        assert_eq!(e.status() & 3, 3, "WEL remains set during WIP");
        let original = e.clone();
        let mut previous = e.bytes(Time::ZERO);
        for us in (0..=2500).step_by(125) {
            let bytes = e.bytes(Time::from_micros(us));
            for index in [0, 127] {
                assert_eq!(bytes[index] & previous[index], bytes[index]);
            }
            previous = bytes;
        }
        assert_eq!([previous[0], previous[127]], [0, 0]);
        for us in (2500..=5000).step_by(125) {
            let bytes = e.bytes(Time::from_micros(us));
            for (index, target) in [(0, 0xff), (127, 0xa5)] {
                assert_eq!(bytes[index] | previous[index], bytes[index]);
                assert_eq!(bytes[index] & target, bytes[index]);
            }
            assert!(bytes[1..127].iter().all(|&v| v == 0xff));
            previous = bytes;
        }
        assert_eq!(e, original, "observation never mutates the write engine");
        for us in [0, 625, 2500, 3750] {
            let mut cut = original.clone();
            let now = Time::from_micros(us);
            let expected = cut.bytes(now);
            cut.power_off(now, &mut ());
            assert_eq!(cut.bytes(Time::from_micros(10000)), expected);
            assert_eq!(cut.status() & 3, 0);
            assert_eq!(cut.deadline(), None);
        }
        e.complete(Time::from_micros(5000), &mut ()).unwrap();
        assert_eq!([e.array[0], e.array[127]], [0xff, 0xa5]);
    }
    #[test]
    fn status_write_has_one_data_byte_and_preserves_visible_status_until_completion() {
        let mut e = M95512::new(&[0; EEPROM_SIZE], 0x80).unwrap();
        command(&mut e, &[6]);
        command(&mut e, &[1, 0x8c, 0]);
        assert_eq!(e.status(), 0x82);
        assert!(!e.busy());
        command(&mut e, &[1, 0x8c]);
        assert_eq!(e.status(), 0x83);
        assert_eq!(e.persistent_status(Time::from_micros(2500)), 0);
        let mut cut = e.clone();
        cut.power_off(Time::from_micros(2500), &mut ());
        assert_eq!(cut.status(), 0);
        assert!(cut.array.iter().all(|&v| v == 0));
        e.complete(Time::from_micros(5000), &mut ()).unwrap();
        assert_eq!(e.status(), 0x8c);
    }
    #[test]
    fn power_up_requires_a_high_select_before_the_first_command() {
        let mut e = M95512::new(&[0xff; EEPROM_SIZE], 0).unwrap();
        e.set_selected(true, Time::ZERO).unwrap();
        xfer(&mut e, 6);
        e.set_selected(false, Time::ZERO).unwrap();
        assert_eq!(e.status(), 0);
        command(&mut e, &[6]);
        assert_eq!(e.status(), 2);
    }
}
