//! Fixed board composition and one exact-horizon execution kernel.
//!
//! The starter retains partial CPU and serial work. Its hardware coverage and
//! timing witnesses are enumerated in docs/STATUS.md; successful execution is
//! not a claim of complete silicon conformance.
use crate::{
    cpu::{alu::I, Action, Cpu, Registers, Width},
    devices::{bma150::Bma150, m95512::M95512, nt7508::Nt7508},
    error::Error,
    mcu::{
        clocks::{ClockWait, Frequencies, Tap},
        gpio::SerialLevels,
        Mcu,
    },
    signals::{Drive, Event, Input, Output, Piezo, TimedInput},
    time::{Time, TimeError},
};

#[derive(Clone, Copy, Debug)]
pub struct Images<'a> {
    pub firmware: &'a [u8],
    pub eeprom: &'a [u8],
    pub eeprom_status: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    pub clocks: Frequencies,
    pub supply_millivolts: u16,
    pub temperature_millicelsius: i32,
    /// Uncalibrated board transfer witness, not a measured Pokewalker constant.
    pub adc_reference_millivolts: u16,
}
impl Default for Conditions {
    fn default() -> Self {
        Self {
            clocks: Frequencies::default(),
            supply_millivolts: 3000,
            temperature_millicelsius: 20_000,
            adc_reference_millivolts: 3300,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunResult {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    action: Action,
    wait: ClockWait,
    /// Word accesses to byte-wide SFRs retain their first completed lane.
    split: bool,
    lane: u8,
    high: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resume {
    Sleep(ClockWait),
    Wake { wait: ClockWait, direct: bool },
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
    now: Time,
    last_effect: Time,
    cpu: Cpu,
    mcu: Mcu,
    eeprom: M95512,
    sensor: Bma150,
    lcd: Nt7508,
    conditions: Conditions,
    analog_pins: [Option<u16>; 7],
    pending: Option<Pending>,
    resume_after: Option<Resume>,
    next_devices: Option<Time>,
    serial: SerialLevels,
    piezo: Piezo,
    incident_light: bool,
    emitting: bool,
    reset_asserted: bool,
    watchdog_reset: Option<ClockWait>,
    powered: bool,
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
    /// `None` selects the documented canonical sensor image, not a claim that
    /// all physical units have identical calibration.
    pub fn with_persistent_state(
        images: Images<'_>,
        conditions: Conditions,
        sensor_nonvolatile: Option<&[u8]>,
    ) -> Result<Self, Error> {
        if conditions.adc_reference_millivolts == 0 {
            return Err(Error::BadInput("ADC reference must be positive"));
        }
        let mcu = Mcu::new(images.firmware, conditions.clocks)?;
        let cpu = Cpu::reset();
        let mut m = Self {
            now: Time::ZERO,
            last_effect: Time::ZERO,
            cpu,
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
            serial: SerialLevels::default(),
            piezo: Piezo::Neutral,
            incident_light: false,
            emitting: false,
            reset_asserted: false,
            watchdog_reset: None,
            powered: conditions.supply_millivolts != 0,
            fault: None,
            stats: Statistics::default(),
        };
        m.sensor
            .set_temperature(conditions.temperature_millicelsius);
        if m.powered {
            m.resolve_board(&mut ())?;
        }
        m.refresh_deadline()?;
        Ok(m)
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
        self.cpu.phase_name()
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
    pub fn firmware(&self) -> &[u8; 49_152] {
        self.mcu.firmware()
    }
    pub fn ram(&self) -> &[u8; 2048] {
        self.mcu.ram()
    }
    pub fn eeprom(&self) -> [u8; 65_536] {
        self.eeprom.bytes(self.now)
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
    pub fn display(&self, pixels: &mut [u8; 6144]) {
        if self.powered {
            self.lcd.render(pixels);
        } else {
            pixels.fill(0);
        }
    }
    pub fn display_enabled(&self) -> bool {
        self.powered && self.lcd.enabled()
    }
    pub fn display_drive(&self) -> Result<crate::devices::nt7508::LcdDrive, Error> {
        if !self.powered {
            return Ok(crate::devices::nt7508::LcdDrive::OFF);
        }
        let at = Time::from_raw(self.now.raw().saturating_sub(1)).max(self.last_effect);
        self.lcd.drive(at)
    }
    pub fn display_start_line(&self) -> u8 {
        self.lcd.start_line()
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
        if Mcu::is_memory(address) || !self.powered {
            return self.mcu.peek8(address);
        }
        let mut view = self.mcu.clone();
        // Exclude effects exactly at the caller's unprocessed horizon.
        let t = Time::from_raw(self.now.raw().saturating_sub(1)).max(self.last_effect);
        view.sync(t)?;
        view.peek8(address)
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.clone(),
        }
    }
    pub fn restore(&mut self, snapshot: &Snapshot) {
        *self = snapshot.state.clone();
    }
    /// Typed in-memory snapshots include in-flight work; this is not a portable
    /// serialization promise. The caller may use equality for same-run snapshots.
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        snapshot.state.clone()
    }

    pub fn powered(&self) -> bool {
        self.powered
    }
    /// Remove board power, retaining partially programmed nonvolatile cells.
    /// ResetPin is a different MCU-only input and leaves external chips powered.
    pub fn power_off(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        if !self.powered {
            return Ok(());
        }
        if self.next_devices == Some(self.now) {
            self.devices_at_boundary(out)?;
        }
        self.eeprom.power_off(self.now, out);
        self.sensor.power_off(self.now, out);
        self.last_effect = self.now;
        self.mcu
            .sci
            .set_power(false, false, false, false, self.now, &self.mcu.clocks)?;
        if self.emitting {
            self.emitting = false;
            out.event(Event::Infrared {
                at: self.now,
                emitting: false,
            });
        }
        if self.piezo != Piezo::Neutral {
            self.piezo = Piezo::Neutral;
            out.event(Event::Buzzer {
                at: self.now,
                drive: Piezo::Neutral,
            });
        }
        self.powered = false;
        self.watchdog_reset = None;
        self.pending = None;
        self.resume_after = None;
        self.next_devices = None;
        out.event(Event::Power {
            at: self.now,
            on: false,
        });
        Ok(())
    }
    pub fn power_on(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        if self.powered || self.conditions.supply_millivolts == 0 {
            return Ok(());
        }
        if !self.reset_asserted && !self.mcu.control.nmi_level() {
            return Err(Self::boot_strap_error());
        }
        self.last_effect = self.now;
        self.mcu.power_on(self.now, out)?;
        self.mcu.hold_reset(self.reset_asserted, self.now, out)?;
        self.sensor.power_on(self.now);
        self.lcd = Nt7508::new();
        self.cpu = Cpu::reset();
        self.serial = SerialLevels::default();
        self.powered = true;
        self.fault = None;
        self.pending = None;
        self.resume_after = None;
        out.event(Event::Power {
            at: self.now,
            on: true,
        });
        self.resolve_board(out)?;
        self.refresh_deadline()
    }
    fn refresh_deadline(&mut self) -> Result<(), Error> {
        if !self.powered {
            self.next_devices = None;
            return Ok(());
        }
        self.next_devices = [
            self.mcu.deadline()?,
            self.eeprom.deadline(),
            self.sensor.deadline(),
            self.watchdog_reset
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
    fn serial_data(&self) -> Result<[Option<bool>; 2], Error> {
        let miso = match (self.eeprom.output(), self.sensor.output()) {
            (Drive::Low, Drive::High) | (Drive::High, Drive::Low) => {
                return Err(Error::Unsupported {
                    component: "board serial net",
                    detail: "opposing external push-pull drivers",
                    address: 0xffdc,
                });
            }
            (Drive::Low, _) | (_, Drive::Low) => Some(false),
            (Drive::High, _) | (_, Drive::High) => Some(true),
            _ => None,
        };
        let mosi = match self.sensor.data_output() {
            Drive::Low => Some(false),
            Drive::High => Some(true),
            Drive::Floating => None,
        };
        Ok([mosi, miso])
    }
    fn resolve_board(&mut self, out: &mut dyn Output) -> Result<[bool; 4], Error> {
        let sci_pins = self.mcu.sci.pins();
        self.mcu.gpio.set_sci_pins(sci_pins, self.incident_light);
        self.mcu
            .gpio
            .set_aec_output(self.mcu.aec.pwm_enabled(), self.mcu.aec.pwm_output());
        let pins = self.mcu.ssu.pins();
        let timer = self.mcu.timer_w.outputs() << 1;
        let timer_mask = self.mcu.timer_w.drives() << 1;
        let data = self.serial_data()?;
        let levels = self.mcu.gpio.resolve(pins, timer, timer_mask, data);
        let sampled = self.mcu.gpio.serial_inputs();
        self.eeprom.set_selected(levels.eeprom_selected, self.now)?;
        self.sensor.set_selected(levels.sensor_selected);
        self.lcd.select(levels.lcd_selected);
        self.lcd.command_data(levels.data);
        if levels.clock != self.serial.clock {
            if levels.clock {
                self.eeprom.rising(levels.mosi);
                self.sensor.rising(levels.mosi, self.now)?;
                self.lcd.rising(levels.mosi, self.now, out)?;
            } else {
                self.eeprom.falling();
                self.sensor.falling();
            }
        }
        let data = self.serial_data()?;
        self.mcu.gpio.resolve(pins, timer, timer_mask, data);
        self.serial = levels;
        let serial = self.mcu.gpio.serial_inputs();
        if let Some(edge) = self.mcu.ssu.input_pins(serial[0], serial[1]) {
            if edge.sample {
                self.mcu.ssu.sample(sampled[self.mcu.ssu.input_pin()]);
            }
            self.mcu.ssu.finish_edge(self.now, &self.mcu.clocks)?;
        }
        // Selection and an external clock edge can change the SSU's output
        // drivers. Resolve that electrical consequence at this same instant.
        let (sck, rxd) = self.mcu.gpio.sci_inputs();
        self.mcu
            .sci
            .input_pins(sck, rxd, self.now, &self.mcu.clocks)?;
        if self.mcu.ssu.pins() != pins || self.mcu.sci.pins() != sci_pins {
            self.resolve_board(out)?;
        }
        let emitting = self.mcu.gpio.emitting();
        if emitting != self.emitting {
            self.emitting = emitting;
            out.event(Event::Infrared {
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
            out.event(Event::Buzzer {
                at: self.now,
                drive,
            });
        }
        if self.mcu.comparators.enabled_mask() != 0 {
            let (pb, vcref) = self.analog_values();
            self.mcu.comparators.set_inputs(
                self.now,
                self.conditions.supply_millivolts,
                vcref,
                [pb[4], pb[5]],
            )?;
        }
        self.mcu
            .timer_w
            .input_pins(self.mcu.gpio.timer_inputs(), self.now, &self.mcu.clocks)?;
        self.mcu
            .aec
            .input_pins(self.mcu.gpio.aec_inputs(), self.now, &self.mcu.clocks)?;
        self.mcu.collect_aec_requests();
        let (selected, high) = self.mcu.gpio.adc_trigger();
        self.mcu.adc.input_trigger(
            selected,
            high,
            self.mcu.control.iegr & 0x20 != 0,
            self.now,
            &self.mcu.clocks,
        )?;
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
            self.reset_asserted || self.watchdog_reset.is_some(),
            self.now,
            out,
        )?;
        self.cpu = Cpu::reset();
        self.pending = None;
        self.resume_after = None;
        self.stats.resets = self.stats.resets.wrapping_add(1);
        out.event(Event::Reset {
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
                if self.mcu.gpio.battery_switch() {
                    supply
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
        let reference = u32::from(self.conditions.adc_reference_millivolts);
        // Figure 17.6 places ideal transitions at half-LSB boundaries.
        Some(
            ((u32::from(pb[usize::from(channel - 4)]) * 2048 + reference) / (2 * reference))
                .min(1023) as u16,
        )
    }
    fn devices_at_boundary(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        self.last_effect = self.now;
        self.stats.peripheral_boundaries = self.stats.peripheral_boundaries.wrapping_add(1);
        // External nonvolatile and sensor clocks do not vanish on an MCU reset.
        if self.eeprom.deadline() == Some(self.now) {
            self.eeprom.complete(self.now, out)?;
        }
        if self.sensor.deadline() == Some(self.now) {
            self.sensor.at_deadline(self.now, out)?;
        }
        let reset = self.mcu.sync(self.now)?;
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
            self.mcu.hold_reset(self.reset_asserted, self.now, out)?;
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
        if self.mcu.ssu.deadline(&self.mcu.clocks)? == Some(self.now) {
            let edge = self.mcu.ssu.advance(self.now, &self.mcu.clocks)?;
            let sampled = self.resolve_board(out)?;
            if let Some(edge) = edge {
                if edge.sample {
                    self.mcu.ssu.sample(sampled[self.mcu.ssu.input_pin()]);
                }
                self.mcu.ssu.finish_edge(self.now, &self.mcu.clocks)?;
            }
        }
        self.resolve_board(out)?;
        self.refresh_deadline()
    }
    fn input_tag(input: Input) -> u8 {
        match input {
            Input::Power(_) => 22,
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
    fn boot_strap_error() -> Error {
        Error::Unsupported {
            component: "reset straps",
            address: 0,
            detail:
                "NMI low at reset release requires unimplemented boot-mode/strap behavior (§6.3)",
        }
    }
    fn apply_batch(&mut self, changes: &[TimedInput], out: &mut dyn Output) -> Result<(), Error> {
        self.last_effect = self.now;
        if self.powered && self.mcu.sync(self.now)? {
            self.reset_mcu(true, out)?;
        }
        let mut ir = None;
        let mut reset = None;
        let mut nmi = None;
        let mut power = None;
        let prior_supply = self.conditions.supply_millivolts;
        for change in changes {
            match change.input {
                Input::Power(on) => power = Some(on),
                Input::Buttons(b) => self.mcu.gpio.set_buttons(b),
                Input::Acceleration(a) => self.sensor.set_input(a)?,
                Input::SupplyMillivolts(v) => self.conditions.supply_millivolts = v,
                Input::TemperatureMillicelsius(v) => {
                    self.conditions.temperature_millicelsius = v;
                    self.sensor.set_temperature(v);
                }
                Input::InfraredLevel(v) => ir = Some(v),
                Input::ResetPin(high) => reset = Some(!high),
                Input::NmiPin(high) => nmi = Some(high),
                Input::AnalogPin { pin, millivolts } => self.analog_pins[pin.index()] = millivolts,
                Input::DigitalPin { pin, level } => self.mcu.gpio.set_digital_level(pin, level),
            }
        }
        if self.conditions.supply_millivolts == 0 {
            power = Some(false);
        } else if prior_supply == 0 && power != Some(false) {
            power = Some(true);
        }
        self.mcu.gpio.set_analog_levels(
            self.analog_pins.map(|v| {
                v.map(|v| u32::from(v) * 2 > u32::from(self.conditions.supply_millivolts))
            }),
        );
        let was_reset = self.reset_asserted;
        let will_reset = reset.unwrap_or(was_reset);
        if let Some(high) = nmi {
            // An edge simultaneous with reset assertion/release is not treated
            // as a user-mode interrupt. Pins in that aperture are reset straps.
            self.mcu.control.nmi_input(
                high,
                self.powered
                    && power != Some(false)
                    && !was_reset
                    && !will_reset
                    && self.watchdog_reset.is_none(),
            );
        }
        if power == Some(false) {
            self.power_off(out)?;
        }
        if let Some(asserted) = reset {
            if self.powered && !asserted && was_reset && !self.mcu.control.nmi_level() {
                return Err(Self::boot_strap_error());
            }
            self.reset_asserted = asserted;
            if self.powered && asserted && !was_reset {
                self.reset_mcu(false, out)?;
            } else if self.powered && !asserted && was_reset {
                self.mcu
                    .hold_reset(self.watchdog_reset.is_some(), self.now, out)?;
            }
        }
        if power == Some(true) {
            self.power_on(out)?;
        }
        if let Some(light) = ir {
            self.incident_light = light;
        }
        if !self.powered {
            return Ok(());
        }
        self.resolve_board(out)?;
        self.refresh_deadline()
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
        // A clock transition is itself an effect boundary. Peripherals which
        // keep running through it must consume the old clock/gate interval,
        // even when no other device happened to schedule an appointment here.
        if self.mcu.sync(self.now)? {
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
    fn queue_cpu(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        if self.pending.is_some()
            || self.reset_asserted
            || self.watchdog_reset.is_some()
            || !self.powered
        {
            return Ok(());
        }
        if let Some(resume) = self.resume_after {
            if self.resume_deadline()? != Some(self.now) {
                return Ok(());
            }
            self.resume_after = None;
            match resume {
                Resume::Sleep(_) => self.enter_sleep(out)?,
                Resume::Wake { direct, .. } => {
                    if self.mcu.sync(self.now)? {
                        self.reset_mcu(true, out)?;
                    } else {
                        self.mcu.control.stabilizing_from = None;
                        self.mcu.apply_gates(self.now, out)?;
                        self.resolve_board(out)?;
                        self.refresh_deadline()?;
                        if direct {
                            self.cpu.direct_transition()?;
                        }
                    }
                }
            }
            if self.resume_after.is_some() {
                return Ok(());
            }
        }
        if self.cpu.sleeping() && self.mcu.control.sleeping() {
            let irq = self.mcu.interrupt();
            if irq.is_none() || (irq != Some(7) && self.cpu.registers.ccr & I != 0) {
                return Ok(());
            }
            if self.mcu.control.sleeping() {
                let wait = self.mcu.control.wake(self.now, &mut self.mcu.clocks)?;
                self.mcu.apply_gates(self.now, out)?;
                self.resolve_board(out)?;
                self.refresh_deadline()?;
                if let Some(wait) = wait {
                    self.resume_after = Some(Resume::Wake {
                        wait,
                        direct: false,
                    });
                    return Ok(());
                }
            }
        }
        loop {
            let action = self.cpu.next(self.mcu.interrupt())?;
            if self.cpu.take_accepted_vector() == Some(7) {
                self.mcu.control.acknowledge_nmi();
            }
            match action {
                Action::Sleep => {
                    if self.mcu.control.sleeping() {
                        return Ok(());
                    }
                    if self.mcu.sync(self.now)? {
                        self.reset_mcu(true, out)?;
                        continue;
                    }
                    if self.mcu.control.sys2 & 8 != 0 {
                        self.resume_after = Some(Resume::Sleep(ClockWait::after(
                            self.now,
                            1,
                            Tap::cpu(),
                            &self.mcu.clocks,
                        )?));
                    } else {
                        self.enter_sleep(out)?;
                    }
                    return Ok(());
                }
                Action::Idle(states) => {
                    self.pending = Some(Pending {
                        action,
                        wait: ClockWait::after(
                            self.now,
                            u64::from(states),
                            Tap::cpu(),
                            &self.mcu.clocks,
                        )?,
                        split: false,
                        lane: 0,
                        high: 0,
                    });
                    return Ok(());
                }
                Action::Read { address, width, .. } | Action::Write { address, width, .. } => {
                    let address = if width == Width::Word {
                        address & !1
                    } else {
                        address
                    };
                    let split = width == Width::Word && !Mcu::native_word(address);
                    let physical_width = if split { Width::Byte } else { width };
                    self.pending = Some(Pending {
                        action,
                        wait: ClockWait::after(
                            self.now,
                            Mcu::access_states(address, physical_width),
                            Tap::cpu(),
                            &self.mcu.clocks,
                        )?,
                        split,
                        lane: 0,
                        high: 0,
                    });
                    return Ok(());
                }
            }
        }
    }
    fn complete_cpu(&mut self, out: &mut dyn Output) -> Result<(), Error> {
        self.last_effect = self.now;
        let mut pending = self
            .pending
            .take()
            .ok_or(Error::Internal("CPU completion without pending access"))?;
        let (address, width, write) = match pending.action {
            Action::Idle(_) => {
                self.cpu.complete(0)?;
                return Ok(());
            }
            Action::Read { address, width, .. } => (address, width, false),
            Action::Write { address, width, .. } => (address, width, true),
            Action::Sleep => return Err(Error::Internal("scheduled SLEEP access")),
        };
        let base = if width == Width::Word {
            address & !1
        } else {
            address
        };
        let a = base.wrapping_add(u16::from(pending.lane));
        let w = if pending.split { Width::Byte } else { width };
        let memory = Mcu::is_memory(a);
        if !memory && self.mcu.sync(self.now)? {
            self.reset_mcu(true, out)?;
            return Ok(());
        }
        let value = if let Action::Write {
            value, mov_byte, ..
        } = pending.action
        {
            let v = if pending.split && pending.lane == 0 {
                value >> 8
            } else {
                value
            };
            match w {
                Width::Byte => {
                    self.mcu
                        .write8(a, v as u8, self.cpu.write_origin(mov_byte), self.now, out)?
                }
                Width::Word => self.mcu.write16(a, v, self.now)?,
            };
            self.stats.bus_writes = self.stats.bus_writes.wrapping_add(1);
            v
        } else {
            self.stats.bus_reads = self.stats.bus_reads.wrapping_add(1);
            match w {
                Width::Byte => u16::from(self.mcu.read8(a, self.now)?),
                Width::Word => self.mcu.read16(a)?,
            }
        };
        #[cfg(feature = "trace")]
        out.event(Event::Bus {
            at: self.now,
            pc: self.cpu.instruction_pc(),
            address: a,
            width: w.bytes(),
            write,
            value,
        });
        #[cfg(not(feature = "trace"))]
        let _ = write;
        if !memory {
            self.resolve_board(out)?;
            self.refresh_deadline()?;
        }
        if pending.split && pending.lane == 0 {
            pending.high = value as u8;
            pending.lane = 1;
            pending.wait = ClockWait::after(
                self.now,
                Mcu::access_states(a.wrapping_add(1), Width::Byte),
                Tap::cpu(),
                &self.mcu.clocks,
            )?;
            self.pending = Some(pending);
        } else {
            let value = if pending.split {
                u16::from_be_bytes([pending.high, value as u8])
            } else {
                value
            };
            self.cpu.complete(value)?;
        }
        Ok(())
    }
    pub fn run_until(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
    ) -> Result<RunResult, Error> {
        if let Some(error) = &self.fault {
            return Err(error.clone());
        }
        self.validate_inputs(end, inputs)?;
        let result = self.run_inner(end, inputs, out);
        if let Err(error) = &result {
            self.fault = Some(error.clone());
        }
        result
    }
    fn run_inner(
        &mut self,
        end: Time,
        inputs: &[TimedInput],
        out: &mut dyn Output,
    ) -> Result<RunResult, Error> {
        let mut consumed = 0;
        while self.now < end {
            // Process causes pending at this exact time before admitting another
            // CPU action. Device-before-input/CPU tie rules are starter witnesses;
            // fine-grained silicon conflict coverage is explicit in STATUS.md.
            if self.next_devices == Some(self.now) {
                self.devices_at_boundary(out)?;
            }
            if consumed < inputs.len() && inputs[consumed].at == self.now {
                let start = consumed;
                while consumed < inputs.len() && inputs[consumed].at == self.now {
                    consumed += 1;
                }
                self.apply_batch(&inputs[start..consumed], out)?;
            }
            if self.pending_deadline()? == Some(self.now) {
                self.complete_cpu(out)?;
            }
            self.queue_cpu(out)?;
            let next = [
                Some(end),
                self.pending_deadline()?,
                self.next_devices,
                inputs.get(consumed).map(|i| i.at),
                self.resume_deadline()?,
            ]
            .into_iter()
            .flatten()
            .min()
            .ok_or(Error::Internal("no next time"))?;
            if next <= self.now {
                return Err(Error::Internal("non-advancing event loop"));
            }
            self.now = next;
        }
        Ok(RunResult {
            now: self.now,
            inputs_consumed: consumed,
            retired: self.cpu.retired,
        })
    }
}
