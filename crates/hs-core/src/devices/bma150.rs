//! BMA150 digital signal, register and four-wire serial owner.
//!
//! The moving-average window mapping is a nominal behavioral model. Analog
//! transfer, undocumented 0x1e effects, filter rounding and axis publication
//! skew are NOT certified silicon behavior; see docs/STATUS.md. There is one
//! model, not a raw-register shortcut beside a physical-input model.
use super::nv::WriteCycle;
use crate::{
    error::Error,
    signals::{Acceleration, Drive, Event, NvDomain, Output},
    time::{Clock, Duration, Time},
};
const COUNT: usize = 0x3e;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Serial {
    Address,
    Read(u8),
    Write(u8),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bma150 {
    registers: [u8; COUNT],
    nonvolatile: [u8; 0x13],
    serial: Serial,
    selected: bool,
    rx: u8,
    rx_bits: u8,
    tx: Option<u8>,
    tx_bit: u8,
    driven: Drive,
    shadows: [Option<u8>; 3],
    input: Acceleration,
    history: [[i16; 3]; 64],
    history_at: u8,
    fill: u8,
    sample_clock: Clock,
    wake_deadline: Option<Time>,
    nv_operation: Option<(WriteCycle, u8, u8)>,
    image_deadline: Option<Time>,
    motion_history: [[i16; 3]; 3],
    motion_at: u8,
    motion_divider: u8,
    motion_set: u8,
    motion_clear: u8,
    data_ready: bool,
    motion_irq: bool,
}
impl Bma150 {
    pub fn new(now: Time) -> Self {
        let mut registers = [0; COUNT];
        registers[0] = 2;
        registers[1] = 0x10;
        registers[0x14] = 6;
        registers[0x15] = 0x80;
        let mut nonvolatile = [0; 0x13];
        nonvolatile[0x14 - 0x0b] = 6;
        nonvolatile[0x15 - 0x0b] = 0x80;
        Self {
            registers,
            nonvolatile,
            serial: Serial::Address,
            selected: false,
            rx: 0,
            rx_bits: 0,
            tx: None,
            tx_bit: 8,
            driven: Drive::Floating,
            shadows: [None; 3],
            input: Acceleration::STILL,
            history: [[0; 3]; 64],
            history_at: 0,
            fill: 0,
            sample_clock: Clock::new(now, 3000, 1).expect("constant sensor frequency"),
            wake_deadline: None,
            nv_operation: None,
            image_deadline: None,
            motion_history: [[0; 3]; 3],
            motion_at: 0,
            motion_divider: 0,
            motion_set: 0,
            motion_clear: 0,
            data_ready: false,
            motion_irq: false,
        }
    }
    /// Restore the sensor's own 0x2b..=0x3d nonvolatile image before execution.
    /// This is distinct from the external 64 KiB EEPROM. The volatile working
    /// image is loaded exactly as at a modeled cold start.
    pub fn from_nonvolatile(now: Time, bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 0x13 {
            return Err(Error::ImageSize {
                name: "BMA150 nonvolatile image",
                expected: 0x13,
                actual: bytes.len(),
            });
        }
        let mut sensor = Self::new(now);
        sensor.nonvolatile.copy_from_slice(bytes);
        sensor.copy_image();
        Ok(sensor)
    }

    pub fn nonvolatile_busy(&self) -> bool {
        self.nv_operation.is_some()
    }
    pub fn power_off(&mut self, now: Time, output: &mut dyn Output) {
        if let Some((cycle, address, value)) = self.nv_operation.take() {
            let index = usize::from(address - 0x2b);
            let value = cycle.byte(
                self.nonvolatile[index],
                value,
                0x20000 + u32::from(address),
                now,
            );
            self.nonvolatile[index] = value;
            output.event(Event::NvByte {
                at: now,
                domain: NvDomain::Sensor,
                address: u16::from(address),
                value,
            });
            output.event(Event::NvInterrupted {
                at: now,
                domain: NvDomain::Sensor,
                address: u16::from(address),
                length: 1,
            });
        }
        self.set_selected(false);
    }
    pub fn power_on(&mut self, now: Time) {
        let image = self.nonvolatile;
        let input = self.input;
        *self = Self::new(now);
        self.nonvolatile = image;
        self.input = input;
        self.copy_image();
    }
    pub fn set_input(&mut self, input: Acceleration) -> Result<(), Error> {
        if [input.x, input.y, input.z]
            .iter()
            .any(|v| v.unsigned_abs() > 1_000_000_000)
        {
            return Err(Error::BadInput(
                "acceleration exceeds the model's safe numerical input range",
            ));
        }
        self.input = input;
        Ok(())
    }
    pub fn sleeping(&self) -> bool {
        self.registers[0x0a] & 1 != 0
    }
    pub fn interrupt(&self) -> bool {
        self.data_ready || self.motion_irq
    }
    pub fn nonvolatile(&self, now: Time) -> [u8; 0x13] {
        let mut image = self.nonvolatile;
        if let Some((cycle, address, value)) = self.nv_operation {
            let index = usize::from(address - 0x2b);
            image[index] = cycle.byte(image[index], value, 0x20000 + u32::from(address), now);
        }
        image
    }
    pub fn next_sample(&self) -> Option<Time> {
        if self.sleeping() || self.wake_deadline.is_some() {
            None
        } else {
            self.sample_clock.next().ok()
        }
    }
    pub fn deadline(&self) -> Option<Time> {
        [
            self.next_sample(),
            self.wake_deadline,
            self.nv_operation.map(|v| v.0.deadline),
            self.image_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }
    pub fn at_deadline(&mut self, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        if let Some((cycle, address, value)) = self.nv_operation {
            if cycle.deadline == now {
                self.nonvolatile[usize::from(address - 0x2b)] = value;
                self.nv_operation = None;
                self.copy_image();
                output.event(Event::NvByte {
                    at: now,
                    domain: NvDomain::Sensor,
                    address: u16::from(address),
                    value,
                });
                output.event(Event::NvCommit {
                    at: now,
                    domain: NvDomain::Sensor,
                    address: u16::from(address),
                    length: 1,
                });
            }
        }
        if self.image_deadline == Some(now) {
            self.copy_image();
            self.image_deadline = None;
            self.registers[0x0a] &= !0x20;
        }
        if self.wake_deadline == Some(now) {
            self.wake_deadline = None;
            self.sample_clock.rebase(now);
        }
        if self.next_sample() == Some(now) {
            self.sample_clock.advance(1)?;
            self.sample();
        }
        Ok(())
    }
    fn copy_image(&mut self) {
        self.registers[0x0b..=0x1d].copy_from_slice(&self.nonvolatile);
        self.shadows = [None; 3];
    }
    fn range_g(&self) -> i64 {
        match (self.registers[0x14] >> 3) & 3 {
            0 => 2,
            1 => 4,
            _ => 8,
        }
    }
    fn window(&self) -> usize {
        1usize << (6 - (self.registers[0x14] & 7).min(6))
    }
    fn sample(&mut self) {
        let range = self.range_g();
        let input = [self.input.x, self.input.y, self.input.z];
        let mut codes = [0i16; 3];
        for axis in 0..3 {
            codes[axis] =
                ((i64::from(input[axis]) * 512) / (range * 1_000_000)).clamp(-512, 511) as i16;
        }
        self.history[usize::from(self.history_at)] = codes;
        self.history_at = (self.history_at + 1) & 63;
        self.fill = self.fill.saturating_add(1).min(64);
        let window = self.window();
        let count = if usize::from(self.fill) >= window {
            window
        } else {
            1
        };
        let mut filtered = [0i16; 3];
        for (axis, filtered_axis) in filtered.iter_mut().enumerate() {
            let mut sum = 0i32;
            for n in 0..count {
                sum += i32::from(self.history[(usize::from(self.history_at) + 63 - n) & 63][axis]);
            }
            *filtered_axis = (sum / count as i32) as i16;
            let raw = *filtered_axis as u16 & 0x3ff;
            self.registers[2 + axis * 2] = ((raw & 3) as u8) << 6 | 1;
            self.registers[3 + axis * 2] = (raw >> 2) as u8;
        }
        self.registers[8] = 80; // canonical 20 C input; temperature frontend not yet calibrated.
        if self.registers[0x15] & 0x20 != 0 {
            self.data_ready = true;
        }
        self.motion_divider = self.motion_divider.wrapping_add(1);
        if usize::from(self.motion_divider) >= window {
            self.motion_divider = 0;
            let previous = self.motion_history[usize::from(self.motion_at)];
            self.motion_history[usize::from(self.motion_at)] = filtered;
            self.motion_at = (self.motion_at + 1) % 3;
            let threshold = i32::from(self.registers[0x10]) * 4;
            let active = (0..3).any(|axis| {
                (i32::from(filtered[axis]) - i32::from(previous[axis])).abs() >= threshold
            });
            let need = [1, 3, 5, 7][usize::from(self.registers[0x11] >> 6)];
            if self.registers[0x15] & 0x40 != 0 && self.registers[0x0b] & 0x40 != 0 {
                if active {
                    self.motion_set = self.motion_set.saturating_add(1);
                    self.motion_clear = 0;
                } else {
                    self.motion_set = 0;
                    self.motion_clear = self.motion_clear.saturating_add(1);
                }
                if self.motion_set >= need {
                    self.motion_irq = true;
                }
                if self.motion_clear >= need && self.registers[0x15] & 0x10 == 0 {
                    self.motion_irq = false;
                }
            }
        }
    }
    /// Fixture inspection does not release shadow latches or data-ready state.
    pub fn peek(&self, address: u8) -> Option<u8> {
        let i = usize::from(address);
        if i >= COUNT || self.sleeping() {
            return None;
        }
        if address >= 0x16 && self.registers[0x0a] & 0x10 == 0 {
            return None;
        }
        if (0x23..=0x2a).contains(&address) {
            return None;
        }
        if address >= 0x2b {
            return None; // EEPROM is write-only; read its downloaded image.
        }
        Some(self.registers[i])
    }
    fn read_register(&mut self, address: u8) -> Option<u8> {
        let mut v = self.peek(address)?;
        if (2..=7).contains(&address) {
            let axis = usize::from((address - 2) / 2);
            let lo = 2 + axis * 2;
            if self.registers[0x15] & 8 == 0 {
                if address & 1 == 0 {
                    self.shadows[axis] = Some(self.registers[lo + 1]);
                } else {
                    v = self.shadows[axis].take().unwrap_or(self.registers[lo + 1]);
                }
            }
            self.registers[lo] &= !1;
            self.data_ready = false;
        }
        Some(v)
    }
    fn write_register(&mut self, address: u8, value: u8, now: Time) -> Result<(), Error> {
        let i = usize::from(address);
        if i >= COUNT || address <= 9 {
            return Ok(());
        }
        if self.sleeping() && address != 0x0a {
            return Ok(());
        }
        if address >= 0x16 && self.registers[0x0a] & 0x10 == 0 {
            return Ok(());
        }
        if address >= 0x2b {
            if self.nv_operation.is_some() {
                return Ok(()); // The programming engine is occupied.
            }
            self.nv_operation = Some((
                WriteCycle::start(now, Duration::from_millis(28))?,
                address,
                value,
            ));
            return Ok(());
        }
        match address {
            0x0a => {
                if value & 0x0c != 0 {
                    return Err(Error::Unsupported {
                        component: "BMA150",
                        detail: "electrostatic/interrupt self-test has not been characterized",
                        address: 0x0a,
                    });
                }
                if value & 2 != 0 {
                    self.copy_image();
                    self.registers[0x0a] = 0;
                    self.fill = 0;
                    self.shadows = [None; 3];
                    self.wake_deadline = Some(
                        now.checked_add(Duration::from_millis(1))
                            .ok_or(crate::time::TimeError::Overflow)?,
                    );
                } else {
                    let was_asleep = self.sleeping();
                    self.registers[i] = value & 0x31;
                    if value & 0x40 != 0 {
                        self.data_ready = false;
                        self.motion_irq = false;
                        self.motion_set = 0;
                        self.motion_clear = 0;
                    }
                    if was_asleep && value & 1 == 0 {
                        self.fill = 0;
                        self.shadows = [None; 3];
                        self.wake_deadline = Some(
                            now.checked_add(Duration::from_millis(1))
                                .ok_or(crate::time::TimeError::Overflow)?,
                        );
                    }
                    if value & 0x20 != 0 {
                        self.image_deadline = Some(
                            now.checked_add(Duration::from_micros(300))
                                .ok_or(crate::time::TimeError::Overflow)?,
                        );
                    }
                }
            }
            0x14 => {
                if value & 7 == 7 || (value >> 3) & 3 == 3 {
                    return Err(Error::Unsupported {
                        component: "BMA150",
                        detail: "reserved bandwidth or range code",
                        address: 0x14,
                    });
                }
                self.registers[i] = value;
            }
            0x15 => {
                if value & 0x80 == 0 {
                    return Err(Error::Unsupported {
                        component: "BMA150",
                        detail: "three-wire SPI turnaround is not implemented",
                        address: 0x15,
                    });
                }
                if value & 1 != 0 {
                    return Err(Error::Unsupported {
                        component: "BMA150",
                        detail: "autonomous wake-pause algorithm is not implemented",
                        address: 0x15,
                    });
                }
                self.registers[i] = value;
                if value & 8 != 0 {
                    self.shadows = [None; 3];
                }
            }
            0x0b => {
                if value & 0x83 != 0 {
                    return Err(Error::Unsupported {
                        component: "BMA150",
                        detail: "low-g/high-g/alert qualifier is not implemented",
                        address: 0x0b,
                    });
                }
                self.registers[i] = value;
            }
            _ => self.registers[i] = value,
        }
        Ok(())
    }
    pub fn set_selected(&mut self, selected: bool) {
        if self.selected != selected {
            self.selected = selected;
            self.serial = Serial::Address;
            self.rx = 0;
            self.rx_bits = 0;
            self.tx = None;
            self.tx_bit = 8;
            self.driven = Drive::Floating;
        }
    }
    pub fn output(&self) -> Drive {
        if self.selected {
            self.driven
        } else {
            Drive::Floating
        }
    }
    pub fn falling(&mut self) -> Drive {
        self.driven = match self.tx {
            Some(byte) if self.selected && self.tx_bit < 8 => {
                let bit = byte & (0x80 >> self.tx_bit) != 0;
                self.tx_bit += 1;
                if bit {
                    Drive::High
                } else {
                    Drive::Low
                }
            }
            _ => Drive::Floating,
        };
        self.driven
    }
    pub fn rising(&mut self, mosi: bool, now: Time) -> Result<(), Error> {
        if !self.selected {
            return Ok(());
        }
        self.rx = self.rx << 1 | u8::from(mosi);
        self.rx_bits += 1;
        if self.rx_bits != 8 {
            return Ok(());
        }
        let value = self.rx;
        self.rx = 0;
        self.rx_bits = 0;
        self.serial = match self.serial {
            Serial::Address => {
                if value & 0x80 == 0 {
                    Serial::Write(value & 0x7f)
                } else {
                    self.tx = self.read_register(value & 0x7f);
                    self.tx_bit = 0;
                    Serial::Read(value & 0x7f)
                }
            }
            Serial::Read(address) => {
                let next = address.wrapping_add(1) & 0x7f;
                self.tx = self.read_register(next);
                self.tx_bit = 0;
                Serial::Read(next)
            }
            Serial::Write(address) => {
                self.write_register(address, value, now)?;
                Serial::Write(address.wrapping_add(1) & 0x7f)
            }
        };
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn xfer(b: &mut Bma150, v: u8) -> u8 {
        let mut r = 0;
        for n in (0..8).rev() {
            r = (r << 1) | u8::from(b.falling() != Drive::Low);
            b.rising(v & (1 << n) != 0, Time::ZERO).unwrap();
        }
        r
    }
    fn write(b: &mut Bma150, a: u8, v: u8) {
        b.set_selected(true);
        xfer(b, a);
        xfer(b, v);
        b.set_selected(false);
    }
    #[test]
    fn protected_window_and_identity() {
        let mut b = Bma150::new(Time::ZERO);
        assert_eq!(b.peek(0), Some(2));
        assert_eq!(b.peek(0x1e), None);
        write(&mut b, 0x0a, 0x10);
        write(&mut b, 0x1e, 0x80);
        assert_eq!(b.peek(0x1e), Some(0x80));
        write(&mut b, 0x0a, 0);
        assert_eq!(b.peek(0x1e), None);
    }
    #[test]
    fn physical_force_and_paired_shadow() {
        let mut b = Bma150::new(Time::ZERO);
        b.sample();
        assert_eq!(b.peek(7), Some(64));
        let lo = b.read_register(6).unwrap();
        b.set_input(Acceleration {
            x: 0,
            y: 0,
            z: -1_000_000,
        })
        .unwrap();
        b.sample();
        assert_eq!(lo & 0xc0, 0);
        assert_eq!(b.read_register(7), Some(64));
        b.read_register(6);
        assert_eq!(b.read_register(7), Some(192));
    }
    #[test]
    fn nonvolatile_completion_reloads_the_whole_image_and_power_loss_retains_partial_cells() {
        let mut b = Bma150::new(Time::ZERO);
        write(&mut b, 0x0a, 0x10);
        write(&mut b, 0x12, 0x11);
        write(&mut b, 0x32, 0xa5);
        assert_eq!(b.peek(0x32), None);
        assert_eq!(b.peek(0x12), Some(0x11));
        let mut interrupted = b.clone();
        interrupted.power_off(Time::from_micros(14000), &mut ());
        interrupted.power_on(Time::from_micros(50000));
        assert_eq!(interrupted.nonvolatile(Time::from_micros(50000))[7], 0);
        assert_eq!(interrupted.peek(0x12), Some(0));
        let mut events = Vec::new();
        b.at_deadline(Time::from_micros(28000), &mut events)
            .unwrap();
        assert_eq!(b.peek(0x12), Some(0xa5));
        assert_eq!(b.nonvolatile(Time::from_micros(28000))[7], 0xa5);
        assert!(events.iter().any(|e| matches!(
            e,
            Event::NvCommit {
                domain: NvDomain::Sensor,
                ..
            }
        )));
    }
}
