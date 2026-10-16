//! BMA150 conversion, filtering, register and serial state. Physical inputs
//! pass through calibrated ADC codes and the same filter used by interrupts.
mod analog;
mod control;
mod filter;
mod i2c;
mod interrupts;
use super::nv::WriteCycle;
use crate::{
    error::Error,
    signals::{Acceleration, Drive, Event, NvDomain, Output},
    time::{Clock, Duration, Time},
};
const COUNT: usize = 0x3e;
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Serial {
    Address = 0,
    Read(u8) = 1,
    Write(u8) = 2,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Bma150 {
    registers: [u8; COUNT],
    nonvolatile: [u8; 0x13],
    serial: Serial,
    i2c: i2c::State,
    selected: bool,
    rx: u8,
    rx_bits: u8,
    tx: Option<u8>,
    tx_bit: u8,
    driven: Drive,
    shadows: [Option<[u8; 2]>; 3],
    last_msb: [u8; 3],
    tx_pair: Option<[u8; 2]>,
    tx_shadow: bool,
    turnaround: bool,
    input: Acceleration,
    analog: [analog::Axis; 3],
    temperature_millicelsius: i32,
    filter: filter::Filter,
    filtered: [i16; 3],
    sample_clock: Clock,
    wake_deadline: Option<Time>,
    pause_deadline: Option<Time>,
    quiet_deadline: Option<Time>,
    irq_hold: Option<Time>,
    asleep: bool,
    auto_cycles: u16,
    test_phases: Option<u8>,
    nv_operation: Option<(WriteCycle, u8, u8)>,
    image_deadline: Option<Time>,
    interrupts: interrupts::Interrupts,
    data_ready: bool,
    unpowered_since: Option<Time>,
    cold: bool,
    serial_ready: Option<Time>,
}
impl Bma150 {
    pub fn new(now: Time) -> Self {
        let mut registers = [0; COUNT];
        registers[0] = 2;
        registers[1] = 0x10;
        // Register-map defaults; offset binary 512 is the modeled calibrated
        // unit's fixed physical baseline, including after EEPROM reloads.
        let nonvolatile = [
            3, 20, 150, 160, 150, 0, 0, 162, 13, 0x0e, 0x80, 0, 0, 0, 0, 128, 128, 128, 0,
        ];
        registers[0x0b..=0x1d].copy_from_slice(&nonvolatile);
        let mut interrupts = interrupts::Interrupts::default();
        interrupts.configure(&registers);
        Self {
            registers,
            nonvolatile,
            serial: Serial::Address,
            i2c: i2c::State::default(),
            selected: false,
            rx: 0,
            rx_bits: 0,
            tx: None,
            tx_bit: 8,
            driven: Drive::Floating,
            shadows: [None; 3],
            last_msb: [0; 3],
            tx_pair: None,
            tx_shadow: false,
            turnaround: false,
            input: Acceleration::STILL,
            analog: [analog::Axis::new(now, 0); 3],
            temperature_millicelsius: 20_000,
            filter: filter::Filter::new(6),
            filtered: [0; 3],
            sample_clock: Clock::new(now, 12000, 1).expect("constant sensor frequency"),
            wake_deadline: now.checked_add(control::acquisition_delay(Duration::from_millis(3))),
            pause_deadline: None,
            quiet_deadline: None,
            irq_hold: None,
            asleep: false,
            auto_cycles: 0,
            test_phases: None,
            nv_operation: None,
            image_deadline: None,
            interrupts,
            data_ready: false,
            unpowered_since: None,
            cold: false,
            serial_ready: None,
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
    pub fn power_off(&mut self, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        if self.unpowered_since.is_none() && !self.asleep {
            self.hold_analog(now)?;
        }
        if let Some((cycle, address, value)) = self.nv_operation.take() {
            let index = usize::from(address - 0x2b);
            let value = cycle.byte(
                self.nonvolatile[index],
                value,
                0x20000 + u32::from(address),
                now,
            );
            self.nonvolatile[index] = value;
            let _ = output.event(Event::NvByte {
                at: now,
                domain: NvDomain::Sensor,
                address: u16::from(address),
                value,
            });
            let _ = output.event(Event::NvInterrupted {
                at: now,
                domain: NvDomain::Sensor,
                address: u16::from(address),
                length: 1,
            });
        }
        self.set_selected(false);
        self.i2c.abort();
        self.unpowered_since.get_or_insert(now);
        self.cold = true;
        Ok(())
    }
    pub fn power_on(&mut self, now: Time) {
        let image = self.nonvolatile;
        let input = self.input;
        let temperature = self.temperature_millicelsius;
        *self = Self::new(now);
        self.nonvolatile = image;
        self.input = input;
        self.temperature_millicelsius = temperature;
        self.copy_image();
        self.serial_ready = now.checked_add(Duration::from_millis(3));
    }
    pub(crate) fn lose_volatile(&mut self) {
        self.cold = true;
    }
    pub(crate) fn set_supply(
        &mut self,
        rail: u16,
        now: Time,
        output: &mut dyn Output,
    ) -> Result<(), Error> {
        if rail < 2400 {
            if self.unpowered_since.is_none() {
                self.power_off(now, output)?;
                self.cold = rail == 0;
            } else if rail == 0 {
                self.cold = true;
            }
        } else if let Some(at) = self.unpowered_since {
            if self.cold {
                self.power_on(now);
            } else {
                let duration = now
                    .duration_since(at)
                    .ok_or(crate::time::TimeError::Reversed)?;
                for deadline in [
                    &mut self.quiet_deadline,
                    &mut self.pause_deadline,
                    &mut self.irq_hold,
                    &mut self.image_deadline,
                    &mut self.serial_ready,
                ] {
                    *deadline = deadline
                        .map(|t| {
                            t.checked_add(duration)
                                .ok_or(crate::time::TimeError::Overflow)
                        })
                        .transpose()?;
                }
                self.unpowered_since = None;
                if !self.asleep {
                    self.start_acquisition(now, Duration::from_millis(1))?;
                }
            }
        }
        Ok(())
    }
    pub fn set_input(&mut self, input: Acceleration, now: Time) -> Result<(), Error> {
        if [input.x, input.y, input.z]
            .iter()
            .any(|v| v.unsigned_abs() > 1_000_000_000)
        {
            return Err(Error::BadInput(
                "acceleration exceeds the model's safe numerical input range",
            ));
        }
        if self.unpowered_since.is_none() && !self.asleep {
            for (axis, (old, new)) in self.analog.iter_mut().zip(
                [self.input.x, self.input.y, self.input.z]
                    .into_iter()
                    .zip([input.x, input.y, input.z]),
            ) {
                if old != new {
                    axis.advance(now, old)?;
                }
            }
        }
        self.input = input;
        Ok(())
    }
    fn hold_analog(&mut self, now: Time) -> Result<(), Error> {
        for (axis, input) in self
            .analog
            .iter_mut()
            .zip([self.input.x, self.input.y, self.input.z])
        {
            axis.advance(now, input)?;
        }
        Ok(())
    }
    pub fn set_temperature(&mut self, millicelsius: i32) {
        self.temperature_millicelsius = millicelsius;
    }
    pub fn sleeping(&self) -> bool {
        self.asleep
    }
    pub fn interrupt(&self) -> bool {
        self.irq_hold.is_some()
            || (self.data_ready && self.registers[0x15] & 0x20 != 0)
            || self.interrupts.output(&self.registers)
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
        if self.unpowered_since.is_some() || self.sleeping() || self.wake_deadline.is_some() {
            None
        } else {
            self.sample_clock.next().ok()
        }
    }
    pub(crate) fn serial_effect_edges(&self, clock_high: bool) -> u8 {
        if !self.selected {
            return self.i2c.effect_edges();
        }
        // Keep read launch/pair capture, first-bit acknowledgement and the
        // three-wire turnaround explicit. Later bits and incoming address/data
        // bits are local shifts until the byte completes. Every raw edge is
        // still replayed before another owner observes the bus. SPI receives
        // on a rise, so from the clock's level the byte completes at a known
        // edge whatever the MCU's CPOL/CPHA.
        if self.turnaround || matches!(self.serial, Serial::Read(_)) && self.rx_bits == 0 {
            1
        } else {
            (8 - self.rx_bits) * 2 - u8::from(!clock_high)
        }
    }
    pub fn deadline(&self) -> Option<Time> {
        self.next_deadline(true)
    }
    /// Autonomous sleep can release a serial driver. Ordinary conversions only
    /// change retained sensor state until a connected owner observes it.
    pub(crate) fn interaction_deadline(&self) -> Option<Time> {
        self.next_deadline(self.automatic())
    }
    fn next_deadline(&self, samples: bool) -> Option<Time> {
        if self.unpowered_since.is_some() {
            return None;
        }
        [
            self.serial_ready,
            samples.then(|| self.next_sample()).flatten(),
            self.wake_deadline,
            self.pause_deadline,
            self.quiet_deadline,
            self.irq_hold,
            self.nv_operation.map(|v| v.0.deadline),
            self.image_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }
    /// Consume local phases through the original recurrence. An explicit
    /// interaction, including a tied conversion, belongs to board delivery.
    pub(crate) fn sync_local_until(&mut self, end: Time, inclusive: bool) -> Result<(), Error> {
        if self.automatic() {
            return Ok(());
        }
        let within = |at: Time| at < end || (inclusive && at == end);
        if self.interaction_deadline().is_some_and(within) {
            return Err(Error::Internal("unprocessed sensor interaction"));
        }
        if self.next_sample().is_none() {
            return Ok(());
        }
        // Only samples come before the end, and without automatic control
        // each is its conversion phase and the millisecond tick
        // (`at_deadline`).
        let limit = if inclusive {
            Time::from_raw(end.raw().saturating_add(1))
        } else {
            end
        };
        let samples = self.sample_clock.edges_before(limit);
        for _ in 0..samples {
            let now = self.sample_clock.advance(1)?;
            self.sample_phase(now)?;
            if self.sample_clock.ordinal().is_multiple_of(12) {
                self.interrupts.millisecond(&self.registers);
            }
        }
        if samples != 0 {
            self.automatic_control(end, false, false)?;
        }
        Ok(())
    }
    pub fn at_deadline(&mut self, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        if self.serial_ready == Some(now) {
            self.serial_ready = None;
        }
        let was_criterion = self.interrupts.output(&self.registers);
        let hold_ended = self.irq_hold == Some(now);
        if hold_ended {
            self.irq_hold = None;
        }
        if self.quiet_deadline == Some(now) {
            self.quiet_deadline = None;
        }
        if self.pause_deadline == Some(now) {
            self.start_acquisition(now, Duration::from_millis(1))?;
        }
        if let Some((cycle, address, value)) = self.nv_operation {
            if cycle.deadline == now {
                self.nonvolatile[usize::from(address - 0x2b)] = value;
                self.nv_operation = None;
                self.copy_image();
                let _ = output.event(Event::NvByte {
                    at: now,
                    domain: NvDomain::Sensor,
                    address: u16::from(address),
                    value,
                });
                let _ = output.event(Event::NvCommit {
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
            self.sample_clock = Clock::new(now, 12000, 1)?;
        }
        let mut cycle_ended = false;
        if self.next_sample() == Some(now) {
            self.sample_clock.advance(1)?;
            self.sample_phase(now)?;
            cycle_ended = self.sample_clock.ordinal().is_multiple_of(4);
            if self.sample_clock.ordinal().is_multiple_of(12)
                && (!self.automatic() || usize::from(self.auto_cycles) >= self.acquisition_cycles())
            {
                self.interrupts.millisecond(&self.registers);
            }
        }
        self.automatic_control(now, was_criterion, cycle_ended || hold_ended)
    }
    fn copy_image(&mut self) {
        self.registers[0x0b..=0x1d].copy_from_slice(&self.nonvolatile);
        if self.registers[0x15] & 8 != 0 {
            self.shadows = [None; 3];
        }
        self.filter.select(self.registers[0x14] & 7);
        self.interrupts.configure(&self.registers);
    }
    fn range_g(&self) -> i64 {
        match (self.registers[0x14] >> 3) & 3 {
            0 => 2,
            1 => 4,
            _ => 8,
        }
    }
    fn window(&self) -> usize {
        self.filter.window()
    }
    fn sample_phase(&mut self, now: Time) -> Result<(), Error> {
        let phase = (self.sample_clock.ordinal() - 1) & 3;
        if let Some(left) = &mut self.test_phases {
            *left = left.saturating_sub(1);
        }
        if phase == 0 {
            self.registers[8] =
                ((i64::from(self.temperature_millicelsius) + 30_000) / 500).clamp(0, 255) as u8;
            return Ok(());
        }
        if let Some(left) = self.test_phases {
            if phase == 3 && left == 0 {
                self.test_phases = None;
                self.registers[0x0a] &= !4;
                self.registers[9] |= 0x80;
            }
            return Ok(());
        }
        let axis = phase as usize - 1;
        let range = self.range_g();
        let input = [self.input.x, self.input.y, self.input.z];
        let offset = (u16::from(self.registers[0x1a + axis]) << 2)
            | u16::from(self.registers[0x16 + axis] >> 6);
        let acceleration =
            self.analog[axis].advance(now, input[axis])? + (i64::from(offset) - 512) * 31_250;
        let code = if self.registers[0x0a] & 8 != 0 {
            0
        } else {
            ((acceleration * 512) / (range * 1_000_000)).clamp(-512, 511) as i16
        };
        self.filtered[axis] = self.filter.push(axis, code);
        let raw = self.filtered[axis] as u16 & 0x3ff;
        self.registers[2 + axis * 2] = ((raw & 3) as u8) << 6 | 1;
        self.registers[3 + axis * 2] = (raw >> 2) as u8;
        self.interrupts
            .axis(axis, self.filtered[axis], &self.registers);
        if axis != 2 {
            return Ok(());
        }
        if self.registers[0x15] & 0x20 != 0
            && [2, 4, 6].iter().all(|&lo| self.registers[lo] & 1 != 0)
        {
            self.data_ready = true;
        }
        self.auto_cycles = self.auto_cycles.saturating_add(1);
        let acquired = self.acquisition_cycles();
        if !self.automatic() || usize::from(self.auto_cycles) >= acquired {
            let interval = if self.automatic() && usize::from(self.auto_cycles) == acquired {
                1
            } else {
                self.window()
            };
            self.interrupts
                .cycle(self.filtered, interval, &self.registers);
        }
        Ok(())
    }
    /// Fixture inspection does not release shadow latches or data-ready state.
    pub fn peek(&self, address: u8) -> Option<u8> {
        let i = usize::from(address);
        if i >= COUNT
            || self.sleeping()
            || self.quiet_deadline.is_some()
            || (self.image_deadline.is_some()
                && ((0x0b..=0x1d).contains(&address) || address >= 0x2b))
        {
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
        Some(if address == 9 {
            self.registers[9] | self.interrupts.status()
        } else {
            self.registers[i]
        })
    }
    fn prepare_read(&mut self, address: u8) -> Option<u8> {
        self.tx_pair = None;
        self.tx_shadow = false;
        let mut v = self.peek(address)?;
        if (2..=7).contains(&address) {
            let axis = usize::from((address - 2) / 2);
            let lo = 2 + axis * 2;
            if self.registers[0x15] & 8 == 0 {
                self.tx_shadow = true;
                if address & 1 == 0 {
                    let pair =
                        self.shadows[axis].unwrap_or([self.registers[lo], self.registers[lo + 1]]);
                    self.tx_pair = Some(pair);
                    v = pair[0];
                } else {
                    v = self.shadows[axis].map_or(self.last_msb[axis], |pair| pair[1]);
                }
            }
        }
        Some(v)
    }
    fn acknowledge_read(&mut self, address: u8) {
        if self.tx.is_none() || self.sleeping() || self.quiet_deadline.is_some() {
            return;
        }
        if (2..=7).contains(&address) {
            let axis = usize::from((address - 2) / 2);
            let lo = 2 + axis * 2;
            if self.tx_shadow {
                if let Some(mut pair) = self.tx_pair {
                    pair[0] &= !1;
                    self.shadows[axis] = Some(pair);
                } else {
                    self.last_msb[axis] = self.tx.unwrap_or(0);
                    self.shadows[axis] = None;
                }
            }
            self.registers[lo] &= !1;
            self.data_ready = false;
        }
    }
    /// Deselected with the I2C slave idle and no driver: clock edges that
    /// do not move SDI while SCK stays high cannot start a transfer, and
    /// change nothing here.
    pub(crate) fn serially_idle(&self) -> bool {
        !self.selected
            && self.i2c.idle()
            && self.driven == Drive::Floating
            && !matches!(self.serial, Serial::Read(_))
    }
    pub fn set_selected(&mut self, selected: bool) {
        let selected = selected && self.unpowered_since.is_none();
        if self.selected != selected {
            self.i2c.abort();
            self.selected = selected;
            self.serial = Serial::Address;
            self.rx = 0;
            self.rx_bits = 0;
            self.tx = None;
            self.tx_bit = 8;
            self.tx_pair = None;
            self.tx_shadow = false;
            self.turnaround = false;
            self.driven = Drive::Floating;
        }
    }
    pub fn output(&self) -> Drive {
        if self.serial_ready.is_none()
            && self.selected
            && self.four_wire()
            && !self.sleeping()
            && self.quiet_deadline.is_none()
        {
            self.driven
        } else {
            Drive::Floating
        }
    }
    pub fn data_output(&self) -> Drive {
        if !self.selected
            && self.unpowered_since.is_none()
            && self.serial_ready.is_none()
            && self.i2c.low
        {
            Drive::Low
        } else if self.serial_ready.is_none()
            && self.selected
            && !self.four_wire()
            && !self.sleeping()
            && self.quiet_deadline.is_none()
        {
            self.driven
        } else {
            Drive::Floating
        }
    }
    fn four_wire(&self) -> bool {
        self.registers[0x15] & 0x80 != 0
    }
    pub fn falling(&mut self) -> Drive {
        if self.serial_ready.is_some() || self.unpowered_since.is_some() {
            return Drive::Floating;
        }
        if self.four_wire() {
            self.shift_output();
        }
        self.output()
    }
    /// Output launch consumes the converted byte/pair; later falling edges
    /// advance only the retained serial latch.
    pub(crate) fn falling_at(&mut self, now: Time) -> Result<Drive, Error> {
        if self.selected
            && self.four_wire()
            && matches!(self.serial, Serial::Read(_))
            && self.tx_bit == 0
            && self.serial_ready.is_none()
            && self.quiet_deadline.is_none()
        {
            self.sync_local_until(now, true)?;
        }
        Ok(self.falling())
    }
    fn shift_output(&mut self) {
        if let Serial::Read(address) = self.serial {
            if self.tx_bit == 0 {
                self.tx = self.prepare_read(address);
            }
        } else {
            self.driven = Drive::Floating;
            return;
        }
        self.driven = match self.tx {
            Some(byte) if self.selected && self.tx_bit < 8 => {
                let bit = byte & (0x80 >> self.tx_bit) != 0;
                if bit {
                    Drive::High
                } else {
                    Drive::Low
                }
            }
            _ => Drive::Floating,
        };
        self.tx_bit = self.tx_bit.saturating_add(1).min(8);
    }
    /// Board delivery materializes conversion state only when this bit consumes
    /// it. Standalone diagnostic transport retains caller-driven deadlines.
    pub(crate) fn rising_at(&mut self, mosi: bool, now: Time) -> Result<bool, Error> {
        if self.selected && self.serial_ready.is_none() && self.quiet_deadline.is_none() {
            // The first sampled bit acknowledges freshness. Completion can write
            // controls or launch the next three-wire byte. Preserve tied conversions
            // before those consumers, without revisiting them for quiet local bits.
            if self.turnaround
                || self.rx_bits == 7
                || matches!(self.serial, Serial::Read(_)) && self.rx_bits == 0
            {
                self.sync_local_until(now, true)?;
            }
        }
        self.rising_effect(mosi, now)
    }
    pub fn rising(&mut self, mosi: bool, now: Time) -> Result<(), Error> {
        self.rising_effect(mosi, now).map(|_| ())
    }
    /// Completed register writes can change the sensor's appointments. Read
    /// acknowledgement and shifting retain the existing conversion schedule.
    pub(crate) fn rising_effect(&mut self, mosi: bool, now: Time) -> Result<bool, Error> {
        if !self.selected || self.serial_ready.is_some() || self.quiet_deadline.is_some() {
            return Ok(false);
        }
        if self.turnaround {
            self.turnaround = false;
            self.shift_output();
            return Ok(false);
        }
        let mut deadline_changed = false;
        if let Serial::Read(address) = self.serial {
            if self.rx_bits == 0 {
                self.acknowledge_read(address);
            }
        }
        self.rx = self.rx << 1 | u8::from(mosi);
        self.rx_bits += 1;
        if self.rx_bits == 8 {
            let value = self.rx;
            self.rx = 0;
            self.rx_bits = 0;
            self.serial = match self.serial {
                Serial::Address => {
                    if value & 0x80 == 0 {
                        Serial::Write(value & 0x7f)
                    } else {
                        self.tx = None;
                        self.tx_bit = 0;
                        self.turnaround = !self.four_wire();
                        Serial::Read(value & 0x7f)
                    }
                }
                Serial::Read(address) => {
                    let next = address.wrapping_add(1) & 0x7f;
                    self.tx = None;
                    self.tx_bit = 0;
                    Serial::Read(next)
                }
                Serial::Write(address) => {
                    self.write_register(address, value, now)?;
                    deadline_changed = true;
                    Serial::Address
                }
            };
        }
        if !self.four_wire() && !self.turnaround {
            self.shift_output();
        }
        Ok(deadline_changed)
    }
}

impl Bma150 {
    pub(crate) fn validate(&mut self, now: Time) -> Result<(), Error> {
        use crate::state::{future, require};
        self.filter.rebuild()?;
        self.interrupts.validate()?;
        self.i2c.validate(
            !self.selected && self.unpowered_since.is_none() && self.serial_ready.is_none(),
        )?;
        for axis in &self.analog {
            axis.validate(self.unpowered_since.unwrap_or(now))?;
        }
        require(
            self.sample_clock.at <= now,
            "sensor clock is ahead of saved time",
        )?;
        self.sample_clock.validate(
            if self.unpowered_since.is_some() || self.asleep || self.wake_deadline.is_some() {
                self.sample_clock.at
            } else {
                now
            },
        )?;
        require(
            self.rx_bits < 8
                && self.tx_bit <= 8
                && [self.input.x, self.input.y, self.input.z]
                    .iter()
                    .all(|v| v.unsigned_abs() <= 1_000_000_000)
                && match self.serial {
                    Serial::Address => true,
                    Serial::Read(a) | Serial::Write(a) => a < 128,
                }
                && self.filtered.iter().all(|v| (-512..=511).contains(v))
                && self
                    .unpowered_since
                    .is_none_or(|t| self.sample_clock.at <= t && t <= now),
            "invalid sensor state",
        )?;
        if let Some((cycle, address, _)) = self.nv_operation {
            cycle.validate(now)?;
            require(
                (0x2b..=0x3d).contains(&address),
                "invalid sensor nonvolatile address",
            )?;
        }
        if self.unpowered_since.is_none() {
            for at in [
                self.wake_deadline,
                self.pause_deadline,
                self.quiet_deadline,
                self.irq_hold,
                self.image_deadline,
                self.serial_ready,
            ] {
                future(at, now)?;
            }
        }
        future(self.deadline(), now)
    }
}
