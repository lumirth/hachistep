//! BMA150 conversion, filtering, register and serial state. Physical inputs
//! pass through calibrated ADC codes and the same filter used by interrupts.
mod analog;
mod control;
mod filter;
mod i2c;
mod interrupts;
use super::nv::WriteCycle;
use crate::serial::{Bits, Drives};
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
    #[cfg(feature = "profile-work")]
    #[borsh(skip)]
    pub(crate) sample_phases: crate::profile_work::Counter,
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
            #[cfg(feature = "profile-work")]
            sample_phases: Default::default(),
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
        #[cfg(feature = "profile-work")]
        let sample_phases = self.sample_phases.clone();
        let image = self.nonvolatile;
        let input = self.input;
        let temperature = self.temperature_millicelsius;
        *self = Self::new(now);
        #[cfg(feature = "profile-work")]
        {
            self.sample_phases = sample_phases;
        }
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
    pub(crate) fn serial_effect_edges(&self) -> u8 {
        if !self.selected {
            return self.i2c.effect_edges();
        }
        // Keep read launch/pair capture, first-bit acknowledgement and the
        // three-wire turnaround explicit. Later bits and incoming address/data
        // bits are local shifts until the byte completes. Every raw edge is
        // still replayed before another owner observes the bus. Since SPI
        // always receives on a rise, a falling next edge only makes this bound
        // one half-cycle early, independently of the MCU's CPOL/CPHA.
        if self.turnaround || matches!(self.serial, Serial::Read(_)) && self.rx_bits == 0 {
            1
        } else {
            (8 - self.rx_bits) * 2 - 1
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
        while let Some(at) = self.next_sample().filter(|at| within(*at)) {
            self.at_deadline(at, &mut ())?;
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
                && (!self.automatic() || usize::from(self.auto_cycles) >= 2 * self.window())
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
        #[cfg(feature = "profile-work")]
        self.sample_phases.add(1);
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
        let acquired = 2 * self.window();
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
        self.shift_outputs(1);
    }
    fn shift_outputs(&mut self, count: u8) {
        if count == 0 {
            return;
        }
        if let Serial::Read(address) = self.serial {
            if self.tx_bit == 0 {
                self.tx = self.prepare_read(address);
            }
        } else {
            self.driven = Drive::Floating;
            return;
        }
        let last = self.tx_bit.saturating_add(count - 1);
        let bits = self.tx.map(|byte| Bits::output(byte, self.tx_bit, count));
        self.driven = match bits {
            Some(bits) if self.selected && last < 8 => {
                if bits.value & (1 << (count - 1)) != 0 {
                    Drive::High
                } else {
                    Drive::Low
                }
            }
            _ => Drive::Floating,
        };
        self.tx_bit = self.tx_bit.saturating_add(count).min(8);
    }
    pub(crate) fn quiet_serial(&self) -> bool {
        if !self.selected {
            return self.i2c.idle();
        }
        !self.turnaround
            && (!matches!(self.serial, Serial::Read(_)) || self.rx_bits != 0 && self.tx_bit != 0)
    }
    pub(crate) fn output_planes(&self, rises: u16, falls: u16, lanes: u16) -> [Drives; 2] {
        let events = if self.four_wire() { falls } else { rises };
        let plane = if !self.selected
            || self.serial_ready.is_some()
            || self.quiet_deadline.is_some()
            || self.sleeping()
        {
            Drives::default()
        } else if let (Serial::Read(_), Some(byte)) = (self.serial, self.tx) {
            Drives::launches(byte, self.tx_bit, events, self.driven, lanes)
        } else {
            Drives::default()
        };
        if self.four_wire() {
            [Drives::default(), plane]
        } else {
            [plane, Drives::default()]
        }
    }
    pub(crate) fn shift_falling_prefix(&mut self, falls: u16) {
        if self.serial_ready.is_none() && self.unpowered_since.is_none() && self.four_wire() {
            self.shift_outputs(falls.count_ones() as u8);
        }
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
        self.receive_bits(Bits::one(mosi), now)
    }
    pub(crate) fn receive_bits(&mut self, bits: Bits, now: Time) -> Result<bool, Error> {
        if bits.count == 0
            || !self.selected
            || self.serial_ready.is_some()
            || self.quiet_deadline.is_some()
        {
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
        bits.append(&mut self.rx, &mut self.rx_bits);
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
            self.shift_outputs(bits.count);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_phases_keep_axis_apertures_and_exclusive_endpoints() {
        let mut b = Bma150::new(Time::ZERO);
        b.write_register(0x14, 6, Time::ZERO).unwrap(); // ±2 g, one sample
        b.at_deadline(b.interaction_deadline().unwrap(), &mut ())
            .unwrap();
        assert_eq!(b.interaction_deadline(), None);
        let temperature = b.next_sample().unwrap();
        b.sync_local_until(temperature, false).unwrap();
        assert_eq!(b.peek(8), Some(0));
        b.sync_local_until(temperature, true).unwrap();
        assert_eq!(b.peek(8), Some(100)); // 20 °C
        for (axis, expected) in [[1, 0], [1, 0], [1, 64]].into_iter().enumerate() {
            let at = b.next_sample().unwrap();
            b.sync_local_until(at, false).unwrap();
            assert_eq!(b.peek(2 + axis as u8 * 2), Some(0));
            b.sync_local_until(at, true).unwrap();
            assert_eq!(&b.registers[2 + axis * 2..4 + axis * 2], &expected);
        }
    }
    #[test]
    fn local_sync_leaves_a_tied_image_reload_to_board_delivery() {
        let mut b = Bma150::new(Time::ZERO);
        b.write_register(0x14, 6, Time::ZERO).unwrap(); // ±2 g
        let wake = b.interaction_deadline().unwrap();
        assert!(b.sync_local_until(wake, true).is_err());
        b.at_deadline(wake, &mut ()).unwrap();
        let mut clock = b.sample_clock;
        clock.advance(8).unwrap();
        let finish = clock.at; // Z of the second frame
        let command = Time::from_raw(finish.raw() - Duration::from_micros(300).raw());
        b.sync_local_until(command, true).unwrap();
        b.write_register(0x0a, 0x20, command).unwrap();
        assert_eq!(b.interaction_deadline(), Some(finish));
        b.sync_local_until(finish, false).unwrap();
        assert_eq!(b.registers[7], 64); // prior +1 g at ±2 g
        assert!(b.sync_local_until(finish, true).is_err());
        assert_eq!(b.next_sample(), Some(finish));
        b.at_deadline(finish, &mut ()).unwrap();
        assert_eq!(b.registers[0x14], 0x0e); // image restores ±4 g
        assert_eq!(&b.registers[6..8], &[1, 32]);
    }
    #[test]
    fn undervoltage_preserves_configuration_and_cold_return_qualifies_serial() {
        let mut b = Bma150::new(Time::ZERO);
        write(&mut b, 0x14, 0x05);
        b.set_supply(2000, Time::from_micros(100), &mut ()).unwrap();
        assert_eq!(b.deadline(), None);
        b.set_selected(true);
        assert_eq!(b.output(), Drive::Floating);
        b.set_supply(3000, Time::from_micros(1100), &mut ())
            .unwrap();
        assert_eq!(b.peek(0x14), Some(0x05));
        let first = b.deadline().unwrap();
        assert!(first > Time::from_micros(1700) && first < Time::from_micros(1800));
        b.set_supply(0, Time::from_micros(1200), &mut ()).unwrap();
        b.set_supply(3000, Time::from_micros(2200), &mut ())
            .unwrap();
        assert_eq!(b.peek(0x14), Some(0x0e)); // Factory working image reloaded.
        b.set_selected(true);
        for _ in 0..8 {
            b.rising(true, Time::from_micros(2300)).unwrap();
        }
        assert_eq!(b.output(), Drive::Floating);
        assert_eq!(b.serial, Serial::Address);
        b.set_supply(1, Time::from_micros(2400), &mut ()).unwrap();
        b.lose_volatile();
        b.set_supply(3000, Time::from_micros(1_002_400), &mut ())
            .unwrap();
        assert_eq!(b.peek(0x14), Some(0x0e));
        assert_eq!(
            b.serial_ready,
            Time::from_micros(1_002_400).checked_add(Duration::from_millis(3))
        );
    }
    fn xfer(b: &mut Bma150, v: u8) -> u8 {
        let mut r = 0;
        for n in (0..8).rev() {
            r = (r << 1) | u8::from(b.falling() != Drive::Low);
            b.rising(v & (1 << n) != 0, b.sample_clock.at).unwrap();
        }
        r
    }
    fn write(b: &mut Bma150, a: u8, v: u8) {
        b.set_selected(true);
        xfer(b, a);
        xfer(b, v);
        b.set_selected(false);
    }
    fn read(b: &mut Bma150, address: u8) -> u8 {
        b.set_selected(true);
        xfer(b, address | 0x80);
        let value = xfer(b, 0);
        b.set_selected(false);
        value
    }
    fn sample_cycle(b: &mut Bma150) {
        if let Some(at) = b.wake_deadline {
            b.at_deadline(at, &mut ()).unwrap();
        }
        for _ in 0..4 {
            b.at_deadline(b.next_sample().unwrap(), &mut ()).unwrap();
        }
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
        write(&mut b, 0x14, 6); // ±2 g, unaveraged output.
        sample_cycle(&mut b);
        assert_eq!(b.peek(7), Some(64));
        let lo = read(&mut b, 6);
        b.set_input(
            Acceleration {
                x: 0,
                y: 0,
                z: -1_000_000,
            },
            b.sample_clock.at,
        )
        .unwrap();
        for _ in 0..8 {
            sample_cycle(&mut b);
        }
        assert_eq!(lo & 0xc0, 0);
        assert_eq!(read(&mut b, 7), 64);
        read(&mut b, 6);
        assert_eq!(read(&mut b, 7), 192);
    }
    #[test]
    fn image_reload_preserves_the_pair_held_by_an_acceleration_read() {
        let mut b = Bma150::new(Time::ZERO); // Factory ±4 g.
        sample_cycle(&mut b);
        assert_eq!(read(&mut b, 6), 1);
        b.set_input(
            Acceleration {
                x: 0,
                y: 0,
                z: -1_000_000,
            },
            b.sample_clock.at,
        )
        .unwrap();
        write(&mut b, 0x0a, 0x20); // Reload the same factory configuration.
        while let Some(at) = b.deadline().filter(|at| *at <= Time::from_micros(5000)) {
            b.at_deadline(at, &mut ()).unwrap();
        }
        assert_eq!(
            read(&mut b, 7),
            32,
            "finish the previously captured +1 g pair"
        );
        read(&mut b, 6);
        assert_eq!(read(&mut b, 7), 224, "the next pair sees -1 g");
    }
    #[test]
    fn automatic_new_data_interrupt_acknowledges_without_a_minimum_width() {
        for latch in [0, 0x10] {
            let mut b = Bma150::new(Time::ZERO);
            for (address, value) in [(0x0b, 0), (0x14, 6), (0x15, 0xa1 | latch), (0x0a, 1)] {
                write(&mut b, address, value);
            }
            while !b.interrupt() {
                let at = b.deadline().unwrap();
                assert!(at < Time::from_micros(23000));
                b.at_deadline(at, &mut ()).unwrap();
            }
            // Reading one acceleration byte acknowledges this independent
            // latched source, even at the instant the full vector arrives.
            read(&mut b, 2);
            assert!(!b.interrupt());
        }
    }
    #[test]
    fn spi_control_waits_for_the_eighth_rise_and_reads_keep_first_bit_effects() {
        let mut b = Bma150::new(Time::ZERO);
        b.set_selected(true);
        assert_eq!(b.serial_effect_edges(), 15);
        xfer(&mut b, 0x0a);
        for _ in 0..7 {
            b.falling();
            assert!(!b.rising_effect(false, Time::ZERO).unwrap());
            assert!(!b.sleeping());
        }
        assert_eq!(b.serial_effect_edges(), 1);
        b.falling();
        assert!(b.rising_effect(true, Time::ZERO).unwrap());
        assert!(
            b.sleeping(),
            "sleep is accepted only with the eighth data bit"
        );
        b.set_selected(false);
        write(&mut b, 0x0a, 0);
        write(&mut b, 0x14, 14); // ±4 g, unaveraged output after waking.
        for _ in 0..8 {
            sample_cycle(&mut b);
        }
        b.set_selected(true);
        xfer(&mut b, 0x86);
        assert_eq!(b.serial_effect_edges(), 1);
        b.falling(); // Launch Z LSB and hold its matching +1 g MSB.
        assert_eq!(b.peek(6).unwrap() & 1, 1);
        assert_eq!(b.serial_effect_edges(), 1);
        b.rising(false, b.sample_clock.at).unwrap();
        assert_eq!(b.peek(6).unwrap() & 1, 0);
        assert_eq!(b.serial_effect_edges(), 13);
        b.set_input(
            Acceleration {
                x: 0,
                y: 0,
                z: -1_000_000,
            },
            b.sample_clock.at,
        )
        .unwrap();
        for _ in 0..8 {
            sample_cycle(&mut b);
        }
        for _ in 0..7 {
            b.falling();
            b.rising(false, b.sample_clock.at).unwrap();
        }
        assert_eq!(b.serial_effect_edges(), 1);
        assert_eq!(
            xfer(&mut b, 0),
            32,
            "MSB belongs to the acknowledged +1 g pair"
        );
        b.set_selected(false);
        assert_eq!(read(&mut b, 6) & 0xc0, 0);
        assert_eq!(read(&mut b, 7), 224, "the following pair sees -1 g");
    }
    #[test]
    fn writes_are_address_data_pairs_and_unclocked_read_bytes_have_no_effect() {
        let mut b = Bma150::new(Time::ZERO);
        b.set_selected(true);
        for byte in [0x0c, 0x20, 0x0d, 2] {
            xfer(&mut b, byte);
        }
        b.set_selected(false);
        assert_eq!((b.peek(0x0c), b.peek(0x0d)), (Some(32), Some(2)));
        sample_cycle(&mut b);
        read(&mut b, 2);
        read(&mut b, 3); // next sequential address is Y LSB, but never sampled.
        assert_eq!(b.peek(4).unwrap() & 1, 1);
        b.set_selected(true);
        xfer(&mut b, 0x84); // An address alone is not a read of Y.
        b.falling(); // Launching its first bit is still not an acknowledgement.
        b.set_selected(false);
        assert_eq!(b.peek(4).unwrap() & 1, 1);
        assert_eq!(read(&mut b, 4) & 1, 1);
        assert_eq!(b.peek(4).unwrap() & 1, 0);
    }
    #[test]
    fn three_wire_has_one_turnaround_and_drives_only_sda() {
        let mut b = Bma150::new(Time::ZERO);
        write(&mut b, 0x15, 0);
        b.set_selected(true);
        xfer(&mut b, 0x80);
        assert_eq!(b.data_output(), Drive::Floating);
        b.rising(false, Time::ZERO).unwrap(); // Ninth clock launches D7.
        let mut word = 0u16;
        for _ in 0..16 {
            assert_eq!(b.output(), Drive::Floating);
            word = (word << 1) | u16::from(b.data_output() == Drive::High);
            b.falling();
            b.rising(false, Time::ZERO).unwrap();
        }
        assert_eq!(word, 0x0210);
        b.set_selected(false);
        assert_eq!(b.data_output(), Drive::Floating);
    }
    #[test]
    fn factory_offset_changes_have_the_vendor_calibration_scale_in_each_range() {
        for (range, expected) in [(6, 264u16), (14, 132), (22, 66)] {
            let mut b = Bma150::new(Time::ZERO);
            write(&mut b, 0x14, range);
            write(&mut b, 0x0a, 0x10);
            write(&mut b, 0x18, 0x40); // Z offset 512 -> 513.
            sample_cycle(&mut b);
            assert_eq!(
                u16::from(read(&mut b, 6) >> 6) | (u16::from(read(&mut b, 7)) << 2),
                expected
            );
            assert_eq!(read(&mut b, 8), 100);
        }
    }
    #[test]
    fn axes_publish_sequentially_and_new_data_waits_for_every_axis() {
        let mut b = Bma150::new(Time::ZERO);
        write(&mut b, 0x15, 0xa0);
        b.at_deadline(b.wake_deadline.unwrap(), &mut ()).unwrap();
        b.at_deadline(b.next_sample().unwrap(), &mut ()).unwrap(); // T
        assert_eq!(b.peek(8), Some(100));
        assert_eq!(b.peek(2), Some(0));
        b.at_deadline(b.next_sample().unwrap(), &mut ()).unwrap(); // X
        assert_eq!(read(&mut b, 2) & 1, 1);
        b.at_deadline(b.next_sample().unwrap(), &mut ()).unwrap(); // Y
        b.at_deadline(b.next_sample().unwrap(), &mut ()).unwrap(); // Z
        assert!(!b.interrupt());
        sample_cycle(&mut b);
        assert!(b.interrupt());
        read(&mut b, 2);
        assert!(!b.interrupt());
        b.set_temperature(35_000);
        b.at_deadline(b.next_sample().unwrap(), &mut ()).unwrap();
        assert_eq!(read(&mut b, 8), 130);
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
        interrupted
            .power_off(Time::from_micros(14000), &mut ())
            .unwrap();
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
