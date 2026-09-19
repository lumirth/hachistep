//! Fixed board composition and execution to an exclusive time horizon.
//!
//! Retains partial CPU and serial work across caller horizons. Component timing
//! rules and their evidence are indexed in docs/SOURCES.md.
mod boot;
mod execution;
mod serial;
mod state;
use crate::{
    cpu::{alu::I, Action, Cpu, Registers, Width, WriteOrigin},
    devices::{bma150::Bma150, m95512::M95512, nt7508::Nt7508},
    error::Error,
    mcu::{
        clocks::{ClockWait, Frequencies, Tap},
        gpio::SerialLevels,
        schedule::{self, Appointments},
        Mcu,
    },
    power::Power,
    signals::{Drive, Event, Input, Output, Piezo, TimedInput},
    time::{Time, TimeError},
};
use boot::Boot;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub struct Images<'a> {
    pub firmware: &'a [u8],
    pub eeprom: &'a [u8],
    pub eeprom_status: u8,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    pub clocks: Frequencies,
    pub supply_millivolts: u16,
    pub temperature_millicelsius: i32,
    /// External analog-supply fixture. None connects AVCC to the board supply.
    pub avcc_override_millivolts: Option<u16>,
    /// Nominal effective drop in the P84-switched battery-sensing path.
    pub battery_sense_drop_millivolts: u16,
}
impl Default for Conditions {
    fn default() -> Self {
        Self {
            clocks: Frequencies::default(),
            supply_millivolts: 3000,
            temperature_millicelsius: 20_000,
            avcc_override_millivolts: None,
            battery_sense_drop_millivolts: 600,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunResult {
    /// Exclusive horizon reached, including an earlier return requested by output.
    pub now: Time,
    pub inputs_consumed: usize,
    pub retired: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Statistics {
    pub bus_reads: u64,
    pub bus_writes: u64,
    pub resets: u64,
    pub peripheral_boundaries: u64,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    action: Action,
    wait: ClockWait,
    /// Word accesses to byte-wide SFRs retain their first completed lane.
    split: bool,
    lane: u8,
    high: u8,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Resume {
    Sleep(ClockWait) = 0,
    Wake { wait: ClockWait, direct: bool } = 1,
}
impl Resume {
    fn wait(self) -> ClockWait {
        match self {
            Self::Sleep(wait) | Self::Wake { wait, .. } => wait,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    state: Machine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    firmware_origin: [u8; 32],
    now: Time,
    last_effect: Time,
    cpu: Cpu,
    boot: Option<Boot>,
    mcu: Mcu,
    eeprom: M95512,
    sensor: Bma150,
    lcd: Nt7508,
    conditions: Conditions,
    analog_pins: [Option<u16>; 7],
    pending: Option<Pending>,
    resume_after: Option<Resume>,
    next_devices: Option<Time>,
    appointments: Appointments,
    changed_peripherals: u16,
    serial: SerialLevels,
    piezo: Piezo,
    incident_light: bool,
    emitting: bool,
    reset_asserted: bool,
    reset_release: Option<ClockWait>,
    watchdog_reset: Option<ClockWait>,
    connected: bool,
    power: Power,
    fault: Option<Error>,
    stats: Statistics,
}
impl Machine {
    pub fn new(images: Images<'_>) -> Result<Self, Error> {
        Self::with_conditions(images, Conditions::default())
    }
    pub fn with_conditions(images: Images<'_>, conditions: Conditions) -> Result<Self, Error> {
        Self::with_persistent_state(images, conditions, None)
    }
    /// Construct with a sensor nonvolatile image exported by an earlier session.
    /// `None` constructs the sensor with its default nonvolatile image.
    pub fn with_persistent_state(
        images: Images<'_>,
        conditions: Conditions,
        sensor_nonvolatile: Option<&[u8]>,
    ) -> Result<Self, Error> {
        if conditions.avcc_override_millivolts == Some(0) {
            return Err(Error::BadInput("AVCC fixture must be positive"));
        }
        let mcu = Mcu::new(images.firmware, conditions.clocks)?;
        let cpu = Cpu::reset();
        let mut m = Self {
            firmware_origin: Sha256::digest(images.firmware).into(),
            now: Time::ZERO,
            last_effect: Time::ZERO,
            cpu,
            boot: None,
            mcu,
            eeprom: M95512::new(images.eeprom, images.eeprom_status)?,
            sensor: match sensor_nonvolatile {
                Some(bytes) => Bma150::from_nonvolatile(Time::ZERO, bytes)?,
                None => Bma150::new(Time::ZERO),
            },
            lcd: Nt7508::new(),
            conditions,
            analog_pins: [None; 7],
            pending: None,
            resume_after: None,
            next_devices: None,
            appointments: Appointments::default(),
            changed_peripherals: 0,
            serial: SerialLevels::default(),
            piezo: Piezo::Neutral,
            incident_light: false,
            emitting: false,
            reset_asserted: conditions.supply_millivolts < 1800,
            reset_release: None,
            watchdog_reset: None,
            connected: true,
            power: Power::new(conditions.supply_millivolts)?,
            fault: None,
            stats: Statistics::default(),
        };
        m.sensor
            .set_temperature(conditions.temperature_millicelsius);
        m.mcu.set_supply(m.power.rail, m.now, &mut ())?;
        m.mcu.hold_reset(m.reset_asserted, m.now, &mut ())?;
        m.sensor.set_supply(m.power.rail, m.now, &mut ())?;
        m.lcd.set_supply(m.power.rail, m.now)?;
        m.resolve_board(&mut ())?;
        m.refresh_deadline()?;
        Ok(m)
    }
    /// Identity of the original image, unchanged by guest flash programming.
    pub fn firmware_origin(&self) -> [u8; 32] {
        self.firmware_origin
    }
    pub fn conditions(&self) -> Conditions {
        self.conditions
    }
    pub fn now(&self) -> Time {
        self.now
    }
    pub fn registers(&self) -> &Registers {
        &self.cpu.registers
    }
    pub fn instruction_pc(&self) -> u16 {
        self.cpu.instruction_pc()
    }
    pub fn phase_name(&self) -> &'static str {
        if self.boot.is_some() {
            "boot-service"
        } else {
            self.cpu.phase_name()
        }
    }
    pub fn retired(&self) -> u64 {
        self.cpu.retired
    }
    pub fn interrupt_entries(&self) -> u64 {
        self.cpu.interrupt_entries
    }
    pub fn sleeping(&self) -> bool {
        self.cpu.sleeping()
    }
    pub fn statistics(&self) -> Statistics {
        self.stats
    }
    pub fn firmware(&self) -> Box<[u8; 49_152]> {
        self.mcu.firmware(self.observation_time())
    }
    pub fn ram(&self) -> &[u8; 2048] {
        self.mcu.ram()
    }
    /// Edit RAM at a guest address while execution is stopped. This advances no
    /// clocks and leaves fetched instructions and pending CPU accesses intact.
    /// The complete range must lie in RAM; a rejected edit changes nothing.
    pub fn write_ram(&mut self, address: u16, bytes: &[u8]) -> Result<(), Error> {
        self.check_fault()?;
        let start = address
            .checked_sub(crate::mcu::RAM_START)
            .map(usize::from)
            .ok_or(Error::BadInput("edit address is outside RAM"))?;
        let end = start
            .checked_add(bytes.len())
            .ok_or(Error::BadInput("edit extends beyond RAM"))?;
        self.mcu
            .ram
            .get_mut(start..end)
            .ok_or(Error::BadInput("edit extends beyond RAM"))?
            .copy_from_slice(bytes);
        Ok(())
    }
    pub fn eeprom(&self) -> [u8; 65_536] {
        self.eeprom.bytes(self.now)
    }
    /// Edit EEPROM cells directly, without serial traffic, elapsed time or
    /// persistence events. Programming must finish before editing. Buffered
    /// serial data survives the edit and may subsequently overwrite these cells.
    pub fn write_eeprom(&mut self, address: u16, bytes: &[u8]) -> Result<(), Error> {
        self.check_fault()?;
        self.eeprom.write_bytes(address, bytes)
    }
    /// Whether the EEPROM has an unfinished page or status programming cycle.
    pub fn eeprom_busy(&self) -> bool {
        self.eeprom.busy()
    }
    pub fn eeprom_status(&self) -> u8 {
        self.eeprom.persistent_status(self.now)
    }
    pub fn sensor_nonvolatile(&self) -> [u8; 0x13] {
        self.sensor.nonvolatile(self.now)
    }
    pub fn lcd_ram(&self) -> &[u8; 4096] {
        self.lcd.ram()
    }
    pub fn lcd_icons(&self) -> &[u8; 256] {
        self.lcd.icons()
    }
    /// Row-major 96x64 pixels, with programmed PWM/FRC drive averaged and scaled
    /// to 0..255. Panel tint and response belong to the frontend.
    pub fn display(&self, pixels: &mut [u8; 6144]) {
        self.lcd.render(pixels);
    }
    /// Start an audio stream from the current time and buzzer drive. Feed the
    /// renderer subsequent events and completed run horizons, including silence.
    pub fn audio(&self, sample_rate: u32) -> Result<crate::Audio, Error> {
        crate::Audio::new(sample_rate, self.now, self.piezo)
    }
    pub fn display_enabled(&self) -> bool {
        self.lcd.enabled()
    }
    pub fn display_drive(&self) -> Result<crate::devices::nt7508::LcdDrive, Error> {
        self.lcd.drive(self.observation_time())
    }
    pub fn display_start_line(&self) -> u8 {
        self.lcd.start_line()
    }
    /// Programmed electronic-volume setting, 0..63. The frontend maps this
    /// voltage control to its panel response; it is independent of pixel duty.
    pub fn display_contrast(&self) -> u8 {
        self.lcd.contrast()
    }
    pub fn ssu_counts(&self) -> (u64, u64) {
        (self.mcu.ssu.transmitted, self.mcu.ssu.received)
    }
    pub fn fault(&self) -> Option<&Error> {
        self.fault.as_ref()
    }
    /// Diagnostic inspection is side-effect free. Counter owners are projected
    /// on a temporary copy; guest read side effects are never executed.
    pub fn peek(&self, address: u16) -> Result<u8, Error> {
        if address < 0xc000 {
            return Ok(self.mcu.flash.peek(address, self.observation_time()));
        }
        if Mcu::is_memory(address) || !self.power.mcu() {
            return self.mcu.peek8(address);
        }
        let mut view = self.mcu.clone();
        // Exclude effects exactly at the caller's unprocessed horizon.
        let t = self.observation_time();
        view.sync(t, &mut ())?;
        view.peek8(address)
    }
    fn observation_time(&self) -> Time {
        Time::from_raw(self.now.raw().saturating_sub(1)).max(self.last_effect)
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut state = self.clone();
        state.cpu.retired = 0;
        state.cpu.interrupt_entries = 0;
        state.stats = Statistics::default();
        state.mcu.ssu.transmitted = 0;
        state.mcu.ssu.received = 0;
        state.mcu.sci.transmitted = 0;
        state.mcu.sci.received = 0;
        Snapshot { state }
    }
    /// Atomically restore the same firmware lineage, including all saved NV cells.
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), Error> {
        crate::state::require(
            self.firmware_origin == snapshot.state.firmware_origin,
            "save state belongs to different firmware",
        )?;
        *self = snapshot.state.clone();
        Ok(())
    }
    /// Construct from captured state, including its firmware identity.
    /// Diagnostic counters start at zero after either typed or file restoration.
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        snapshot.state.clone()
    }

    pub fn powered(&self) -> bool {
        self.power.rail != 0
    }
    /// Disconnect the common rail. Physical decay keeps advancing while off.
    pub fn power_off(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        self.set_connected(false, out)
    }
    pub fn power_on(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        self.set_connected(true, out)
    }
    fn set_connected(&mut self, connected: bool, out: &mut dyn Output) -> Result<(), Error> {
        self.check_fault()?;
        if self.connected == connected {
            return Ok(());
        }
        let result = (|| {
            self.settle_boundary(out)?;
            self.connected = connected;
            self.change_rail(
                if connected {
                    self.conditions.supply_millivolts
                } else {
                    0
                },
                None,
                out,
            )
        })();
        self.latch_error(result)
    }
    fn check_fault(&self) -> Result<(), Error> {
        self.fault.clone().map_or(Ok(()), Err)
    }
    fn latch_error<T>(&mut self, result: Result<T, Error>) -> Result<T, Error> {
        if let Err(error) = &result {
            self.fault = Some(error.clone());
        }
        result
    }
    fn settle_boundary(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        if self.next_devices == Some(self.now) {
            self.devices_at_boundary(out)?;
        }
        if self.mcu.sync(self.now, out)? {
            self.reset_mcu(true, out)?;
        }
        Ok(())
    }
    fn change_rail(
        &mut self,
        rail: u16,
        reset_drive: Option<bool>,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        self.last_effect = self.now;
        let old = self.power.rail;
        let old_eeprom = self.power.eeprom();
        self.power.set_rail(rail, self.now)?;
        if let Some(high) = reset_drive {
            self.power.drive_reset(high, self.now)?;
        }
        if old != rail {
            // Interrupt programming before changed MCU drive can raise CS.
            if old_eeprom && !self.power.eeprom() {
                self.eeprom.power_off(self.now, out);
            }
            self.sensor.set_supply(rail, self.now, out)?;
            self.lcd.set_supply(rail, self.now)?;
            self.mcu.set_supply(rail, self.now, out)?;
            if rail < 1800 && self.boot.take().is_some() {
                self.cpu = Cpu::reset();
                self.boot = None;
                self.pending = None;
            }
            if (old != 0) != (rail != 0) {
                let _ = out.event(Event::Power {
                    at: self.now,
                    on: rail != 0,
                });
            }
        }
        self.update_reset(out)?;
        self.resolve_board(out)?;
        self.refresh_deadline()
    }
    fn update_reset(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        if !self.power.mcu() {
            return Ok(());
        }
        let low = self.power.reset_low(self.now);
        if low == self.reset_asserted {
            return Ok(());
        }
        self.reset_asserted = low;
        if low {
            self.reset_release = None;
            self.reset_mcu(false, out)?;
        } else {
            self.reset_release = Some(ClockWait::after(
                self.now,
                8,
                Tap::system(1),
                &self.mcu.clocks,
            )?);
            self.mcu.hold_reset(true, self.now, out)?;
        }
        Ok(())
    }
    fn refresh_deadline(&mut self) -> Result<(), Error> {
        self.refresh_peripherals(schedule::ALL)
    }
    fn refresh_peripherals(&mut self, changed: u16) -> Result<(), Error> {
        let serial = self.serial_deadline()?;
        self.appointments.update(
            changed | self.changed_peripherals | schedule::SSU,
            &self.mcu,
            serial,
        )?;
        self.changed_peripherals = 0;
        self.next_devices = [
            self.power.deadline(),
            self.lcd.deadline(),
            self.appointments.next(),
            self.mcu.clock_output_deadline(self.now)?,
            self.eeprom.deadline(),
            self.sensor.deadline(),
            self.watchdog_reset
                .as_ref()
                .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?,
            self.reset_release
                .as_ref()
                .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?,
        ]
        .into_iter()
        .flatten()
        .min();
        if self.next_devices.is_some_and(|t| t < self.now) {
            return Err(Error::Internal("peripheral appointment in the past"));
        }
        Ok(())
    }
    fn serial_data(&self) -> [Option<bool>; 2] {
        let miso = match (self.eeprom.output(), self.sensor.output()) {
            // Opposing external drivers resolve low in the nominal circuit.
            // The electrical basis is in research/h8-registers-and-gpio.md.
            (Drive::Low, _) | (_, Drive::Low) => Some(false),
            (Drive::High, _) | (_, Drive::High) => Some(true),
            _ => None,
        };
        let mosi = match self.sensor.data_output() {
            Drive::Low => Some(false),
            Drive::High => Some(true),
            Drive::Floating => None,
        };
        [mosi, miso]
    }
    fn resolve_board(&mut self, out: &mut dyn Output) -> Result<[bool; 4], Error> {
        self.settle_board(true, out)
    }
    fn settle_board(
        &mut self,
        configuration_changed: bool,
        out: &mut dyn Output,
    ) -> Result<[bool; 4], Error> {
        let previous = self.mcu.gpio.levels;
        self.mcu.update_clock_output(self.now);
        let sci_pins = self.mcu.sci.pins();
        self.mcu.gpio.set_sci_pins(sci_pins, self.incident_light);
        self.mcu
            .gpio
            .set_aec_output(self.mcu.aec.pwm_enabled(), self.mcu.aec.pwm_output());
        let pins = self.mcu.ssu.pins();
        let timer = self.mcu.timer_w.outputs() << 1;
        let timer_mask = self.mcu.timer_w.drives() << 1;
        let iic_pins = self.mcu.iic.pins();
        self.mcu.gpio.set_iic_pins(iic_pins);
        let data = self.serial_data();
        let levels = if self.power.mcu() {
            self.mcu.gpio.resolve(pins, timer, timer_mask, data)
        } else {
            self.mcu.gpio.resolve_unpowered(data)
        };
        let sampled = self.mcu.gpio.serial_inputs();
        if self.power.eeprom() {
            self.eeprom.set_selected(levels.eeprom_selected, self.now)?;
        }
        self.sensor.set_selected(levels.sensor_selected);
        if !self.serial.sensor_selected
            && !levels.sensor_selected
            && (self.serial.clock != levels.clock || self.serial.mosi != levels.mosi)
        {
            self.sensor.i2c_pins(
                [self.serial.clock, self.serial.mosi],
                [levels.clock, levels.mosi],
                self.now,
            )?;
        }
        self.lcd.select(levels.lcd_selected);
        self.lcd.command_data(levels.data);
        if levels.clock != self.serial.clock {
            if levels.clock {
                if self.power.eeprom() {
                    self.eeprom.rising(levels.mosi);
                }
                self.sensor.rising(levels.mosi, self.now)?;
                self.lcd.rising(levels.mosi, self.now, out)?;
            } else {
                self.eeprom.falling();
                self.sensor.falling();
            }
        }
        let settled_data = self.serial_data();
        // Only the external serial drivers can have changed since the first
        // resolution. Preserve it when the electrical inputs are identical.
        self.serial = if settled_data == data {
            levels
        } else if self.power.mcu() {
            self.mcu.gpio.resolve(pins, timer, timer_mask, settled_data)
        } else {
            self.mcu.gpio.resolve_unpowered(settled_data)
        };
        if !self.power.mcu() {
            if self.emitting {
                self.emitting = false;
                let _ = out.event(Event::Infrared {
                    at: self.now,
                    emitting: false,
                });
            }
            if self.piezo != Piezo::Neutral {
                self.piezo = Piezo::Neutral;
                let _ = out.event(Event::Buzzer {
                    at: self.now,
                    drive: Piezo::Neutral,
                });
            }
            return Ok(sampled);
        }
        let serial = self.mcu.gpio.serial_inputs();
        if let Some(edge) = self.mcu.ssu.input_pins(serial[0], serial[1]) {
            self.changed_peripherals |= schedule::SSU;
            if edge.sample {
                self.mcu.ssu.sample(sampled[self.mcu.ssu.input_pin()]);
            }
            self.mcu.ssu.finish_edge(self.now, &self.mcu.clocks)?;
        }
        // Selection and an external clock edge can change the SSU's output
        // drivers. Resolve that electrical consequence at this same instant.
        if configuration_changed || previous[1] != self.mcu.gpio.levels[1] {
            let (sck, rxd) = self.mcu.gpio.sci_inputs();
            if self
                .mcu
                .sci
                .input_pins(sck, rxd, self.now, &self.mcu.clocks)?
            {
                self.changed_peripherals |= schedule::SCI;
            }
        }
        let [scl, sda] = self.mcu.gpio.iic_inputs();
        if self
            .mcu
            .iic
            .input_pins(scl, sda, self.now, &self.mcu.clocks)?
        {
            self.changed_peripherals |= schedule::IIC;
        }
        if self.mcu.ssu.pins() != pins
            || self.mcu.sci.pins() != sci_pins
            || self.mcu.iic.pins() != iic_pins
        {
            self.resolve_board(out)?;
        }
        let emitting = self.mcu.gpio.emitting();
        if emitting != self.emitting {
            self.emitting = emitting;
            let _ = out.event(Event::Infrared {
                at: self.now,
                emitting,
            });
        }
        self.mcu.control.pins(self.mcu.gpio.irq_levels());
        let (b, c) = self.mcu.gpio.piezo_levels();
        let drive = match (b, c) {
            (true, false) => Piezo::Positive,
            (false, true) => Piezo::Negative,
            _ => Piezo::Neutral,
        };
        if drive != self.piezo {
            self.piezo = drive;
            let _ = out.event(Event::Buzzer {
                at: self.now,
                drive,
            });
        }
        // Clock edges preserve pin routing. Notify analog and timer inputs
        // when their connected ports change; configuration writes revisit all.
        let connected_ports_changed = configuration_changed
            || previous[0] != self.mcu.gpio.levels[0]
            || previous[1] != self.mcu.gpio.levels[1]
            || previous[2] != self.mcu.gpio.levels[2]
            || previous[4] != self.mcu.gpio.levels[4];
        if !connected_ports_changed {
            return Ok(sampled);
        }
        if self.mcu.comparators.enabled_mask() != 0 {
            self.changed_peripherals |= schedule::COMPARATORS;
            let (pb, vcref) = self.analog_values();
            self.mcu.comparators.set_inputs(
                self.now,
                self.conditions.supply_millivolts,
                vcref,
                [pb[4], pb[5]],
            )?;
        }
        if self
            .mcu
            .timer_w
            .input_pins(self.mcu.gpio.timer_inputs(), self.now, &self.mcu.clocks)?
        {
            self.changed_peripherals |= schedule::TIMER_W;
        }
        if self
            .mcu
            .aec
            .input_pins(self.mcu.gpio.aec_inputs(), self.now, &self.mcu.clocks)?
        {
            self.changed_peripherals |= schedule::AEC;
        }
        self.mcu.collect_aec_requests();
        let (selected, high) = self.mcu.gpio.adc_trigger();
        if self.mcu.adc.input_trigger(
            selected,
            high,
            self.mcu.control.iegr & 0x20 != 0,
            self.now,
            &self.mcu.clocks,
        )? {
            self.changed_peripherals |= schedule::ADC;
        }
        Ok(sampled)
    }
    fn reset_mcu(&mut self, watchdog: bool, out: &mut dyn Output) -> Result<(), Error> {
        self.mcu.reset(self.now, watchdog, out)?;
        if watchdog {
            self.watchdog_reset = Some(ClockWait::after(
                self.now,
                512,
                Tap::on_chip(1),
                &self.mcu.clocks,
            )?);
        }
        self.mcu.hold_reset(
            self.reset_asserted || self.reset_release.is_some() || self.watchdog_reset.is_some(),
            self.now,
            out,
        )?;
        self.cpu = Cpu::reset();
        self.boot = None;
        self.pending = None;
        self.resume_after = None;
        self.stats.resets = self.stats.resets.wrapping_add(1);
        let _ = out.event(Event::Reset {
            at: self.now,
            watchdog,
        });
        self.resolve_board(out)?;
        self.refresh_deadline()
    }
    fn analog_values(&self) -> ([u16; 6], u16) {
        let supply = self.conditions.supply_millivolts;
        let buttons = self.mcu.gpio.raw_button_levels();
        let mut pb = [0u16; 6];
        for (i, value) in pb.iter_mut().enumerate() {
            let board = if i == 3 {
                if self
                    .mcu
                    .gpio
                    .battery_switch(self.mcu.timer_w.drives() & 8 != 0)
                {
                    supply.saturating_sub(self.conditions.battery_sense_drop_millivolts)
                } else {
                    0
                }
            } else if buttons & (1 << i) != 0 {
                supply
            } else {
                0
            };
            *value = self.analog_pins[i].unwrap_or(board);
        }
        let vcref = if self.mcu.gpio.external_reference_selected() {
            self.analog_pins[6].unwrap_or(if self.mcu.gpio.levels[1] & 1 != 0 {
                supply
            } else {
                0
            })
        } else {
            0
        };
        (pb, vcref)
    }
    fn analog_code(&self) -> Option<u16> {
        let channel = self.mcu.adc.channel();
        if !(4..=9).contains(&channel) {
            return None;
        }
        let (pb, _) = self.analog_values();
        let reference = u32::from(
            self.conditions
                .avcc_override_millivolts
                .unwrap_or(self.conditions.supply_millivolts),
        );
        if reference == 0 {
            return None;
        }
        // Figure 17.6 places ideal transitions at half-LSB boundaries.
        Some(
            ((u32::from(pb[usize::from(channel - 4)]) * 2048 + reference) / (2 * reference))
                .min(1023) as u16,
        )
    }
    fn devices_at_boundary(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        self.sync_serial_before(self.now, out)?;
        self.last_effect = self.now;
        self.stats.peripheral_boundaries = self.stats.peripheral_boundaries.wrapping_add(1);
        let mut due = self.appointments.due(self.now);
        if due & schedule::STARTUP != 0
            || self.power.deadline() == Some(self.now)
            || self
                .reset_release
                .as_ref()
                .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?
                == Some(self.now)
            || self
                .watchdog_reset
                .as_ref()
                .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?
                == Some(self.now)
        {
            due = schedule::ALL;
        }
        if self.power.deadline() == Some(self.now) {
            if self.power.advance(self.now) {
                self.mcu.lose_volatile(self.now, out)?;
                self.lcd.lose_volatile();
                self.sensor.lose_volatile();
                self.cpu = Cpu::reset();
                self.boot = None;
                self.pending = None;
                self.resume_after = None;
                self.watchdog_reset = None;
                self.reset_release = None;
                self.reset_asserted = true;
                self.mcu.hold_reset(true, self.now, out)?;
            }
            self.update_reset(out)?;
        }
        self.lcd.at_deadline(self.now);
        // External nonvolatile and sensor clocks do not vanish on an MCU reset.
        if self.eeprom.deadline() == Some(self.now) {
            self.eeprom.complete(self.now, out)?;
        }
        if self.sensor.deadline() == Some(self.now) {
            self.sensor.at_deadline(self.now, out)?;
        }
        let reset = self.mcu.sync_peripherals(due, self.now, out)?;
        if reset {
            self.reset_mcu(true, out)?;
        }
        if self
            .watchdog_reset
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?
            == Some(self.now)
        {
            self.watchdog_reset = None;
            self.mcu.hold_reset(
                self.reset_asserted || self.reset_release.is_some(),
                self.now,
                out,
            )?;
        }
        if self
            .reset_release
            .as_ref()
            .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?
            == Some(self.now)
        {
            // TEST shares the ADTRG package input. The fixed board's E7_0
            // boot-enable strap is modeled high; TEST-high is inactive test state.
            let test = self.mcu.gpio.adc_trigger().1;
            if test || !self.mcu.control.nmi_level() {
                self.boot = Some(Boot::new(test));
            }
            self.reset_release = None;
            self.mcu.hold_reset(
                self.reset_asserted || self.watchdog_reset.is_some(),
                self.now,
                out,
            )?;
        }
        if self.mcu.adc.deadline(&self.mcu.clocks)? == Some(self.now)
            && self
                .mcu
                .adc
                .advance(self.now, self.analog_code(), &self.mcu.clocks)?
        {
            self.mcu.control.irr2 |= 0x40;
        }
        if self.mcu.sci.deadline(&self.mcu.clocks)? == Some(self.now) {
            self.mcu.sci.advance(self.now, &self.mcu.clocks)?;
        }
        if self.mcu.iic.deadline(&self.mcu.clocks)? == Some(self.now) {
            self.mcu.iic.advance(self.now, &self.mcu.clocks)?;
        }
        if self.mcu.ssu.deadline(&self.mcu.clocks)? == Some(self.now) {
            self.serial_edge(due == schedule::ALL, out)?;
            due |= schedule::SSU;
        } else {
            self.settle_board(due == schedule::ALL, out)?;
        }
        self.refresh_peripherals(due)
    }
    fn input_tag(input: Input) -> u8 {
        match input {
            Input::Power(_) => 24,
            Input::Buttons(_) => 0,
            Input::Acceleration(_) => 1,
            Input::SupplyMillivolts(_) => 2,
            Input::TemperatureMillicelsius(_) => 23,
            Input::InfraredLevel(_) => 3,
            Input::ResetPin(_) => 4,
            Input::NmiPin(_) => 31,
            Input::AnalogPin { pin, .. } => 5 + pin.index() as u8,
            Input::DigitalPin { pin, .. } => 12 + pin.index() as u8,
        }
    }
    fn validate_inputs(&self, end: Time, inputs: &[TimedInput]) -> Result<(), Error> {
        if end < self.now {
            return Err(TimeError::Reversed.into());
        }
        let mut prior = self.now;
        let mut mask = 0u32;
        for change in inputs {
            if change.at < self.now {
                return Err(Error::PastInput {
                    now: self.now,
                    requested: change.at,
                });
            }
            if change.at < prior {
                return Err(Error::BadInput("input timeline is not ordered"));
            }
            if change.at != prior {
                mask = 0;
            }
            let bit = 1 << Self::input_tag(change.input);
            if mask & bit != 0 {
                return Err(Error::BadInput("duplicate property at one input timestamp"));
            }
            mask |= bit;
            prior = change.at;
            if let Input::Acceleration(a) = change.input {
                if [a.x, a.y, a.z]
                    .iter()
                    .any(|v| i64::from(*v).abs() > 32_000_000)
                {
                    return Err(Error::BadInput(
                        "acceleration exceeds supported +/-32g input envelope",
                    ));
                }
            }
        }
        Ok(())
    }
    fn apply_batch(&mut self, changes: &[TimedInput], out: &mut dyn Output) -> Result<(), Error> {
        self.last_effect = self.now;
        if self.power.mcu() && self.mcu.sync(self.now, out)? {
            self.reset_mcu(true, out)?;
        }
        let mut ir = None;
        let mut reset = None;
        let mut nmi = None;
        let mut power = None;
        for change in changes {
            match change.input {
                Input::Power(on) => power = Some(on),
                Input::Buttons(b) => self.mcu.gpio.set_buttons(b),
                Input::Acceleration(a) => self.sensor.set_input(a, self.now)?,
                Input::SupplyMillivolts(v) => self.conditions.supply_millivolts = v,
                Input::TemperatureMillicelsius(v) => {
                    self.conditions.temperature_millicelsius = v;
                    self.sensor.set_temperature(v);
                }
                Input::InfraredLevel(v) => ir = Some(v),
                Input::ResetPin(high) => reset = Some(high),
                Input::NmiPin(high) => nmi = Some(high),
                Input::AnalogPin { pin, millivolts } => self.analog_pins[pin.index()] = millivolts,
                Input::DigitalPin { pin, level } => self.mcu.gpio.set_digital_level(pin, level),
            }
        }
        if let Some(on) = power {
            self.connected = on;
        }
        let rail = if self.connected {
            self.conditions.supply_millivolts
        } else {
            0
        };
        self.mcu.gpio.set_analog_levels(
            self.analog_pins.map(|v| {
                v.map(|v| u32::from(v) * 2 > u32::from(self.conditions.supply_millivolts))
            }),
        );
        let was_reset = self.reset_asserted;
        let will_reset = reset.map_or(was_reset, |high| !high);
        if let Some(high) = nmi {
            // An edge simultaneous with reset assertion/release is not treated
            // as a user-mode interrupt. Pins in that aperture are reset straps.
            self.mcu.control.nmi_input(
                high,
                self.power.mcu()
                    && rail >= 1800
                    && self.boot.is_none()
                    && !was_reset
                    && !will_reset
                    && self.reset_release.is_none()
                    && self.watchdog_reset.is_none(),
            );
        }
        if let Some(light) = ir {
            self.incident_light = light;
        }
        self.change_rail(rail, reset, out)
    }
    fn pending_deadline(&self) -> Result<Option<Time>, Error> {
        self.pending
            .as_ref()
            .map_or(Ok(None), |p| p.wait.deadline(&self.mcu.clocks))
    }
    fn resume_deadline(&self) -> Result<Option<Time>, Error> {
        self.resume_after
            .map_or(Ok(None), |r| r.wait().deadline(&self.mcu.clocks))
    }
    fn enter_sleep(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        self.sync_serial(out)?;
        // A clock transition is itself an effect boundary. Peripherals which
        // keep running through it must consume the old clock/gate interval,
        // even when no other device happened to schedule an appointment here.
        if self.mcu.sync(self.now, out)? {
            return self.reset_mcu(true, out);
        }
        self.mcu.control.sleep(self.now, &mut self.mcu.clocks)?;
        // The intermediate watch/standby state has real module effects even
        // when a direct transition immediately starts the wake sequence.
        self.mcu.apply_gates(self.now, out)?;
        self.resolve_board(out)?;
        if self.mcu.control.sys2 & 8 != 0 && self.cpu.registers.ccr & I == 0 {
            let wait = self.mcu.control.wake(self.now, &mut self.mcu.clocks)?;
            self.mcu.apply_gates(self.now, out)?;
            self.resolve_board(out)?;
            if let Some(wait) = wait {
                self.resume_after = Some(Resume::Wake { wait, direct: true });
            } else {
                self.cpu.direct_transition()?;
            }
        }
        self.refresh_deadline()
    }
}
