//! Fixed board composition and execution to an exclusive time horizon.
//!
//! Retains partial CPU and serial work across caller horizons. Component timing
//! rules and their evidence are indexed in docs/SOURCES.md.
mod execution;
mod serial;
mod state;
#[cfg(feature = "profile-work")]
mod work;
use crate::{
    cpu::{alu::I, Action, Cpu, Registers, Width},
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
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
enum BoardChange {
    Configuration,
    Peripherals {
        owners: u16,
        clock_output: bool,
        serial_devices: bool,
    },
    Serial,
}

/// Persistent images for a new session. Construction validates and copies the
/// supplied bytes; the resulting machine does not borrow them.
#[derive(Clone, Copy, Debug)]
pub struct Images<'a> {
    /// Raw 49,152-byte H8 internal flash image, including vectors.
    pub firmware: &'a [u8],
    /// Raw 65,536-byte external EEPROM array.
    pub eeprom: &'a [u8],
    /// Persistent M95512 status bits. Transient WIP/WEL bits are rejected.
    pub eeprom_status: u8,
    /// Optional 19-byte BMA150 nonvolatile image. `None` selects the modeled
    /// calibrated default. A supplied image loads the sensor's working registers
    /// as at a cold start; it does not restore an operation in progress.
    pub sensor_nonvolatile: Option<&'a [u8]>,
}
/// Physical conditions at construction. Use timestamped [`Input`] changes for
/// conditions that vary during execution.
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
/// Why a successful execution call returned. Faults return `Error` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// All effects before the requested exclusive horizon have completed.
    Horizon,
    /// The output consumer requested control, including at the last instant.
    Output,
}
/// Successful bounded execution. Resume with the unconsumed input suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunResult {
    /// Exclusive horizon reached, including an earlier return requested by output.
    pub now: Time,
    /// Prefix of the supplied timeline consumed by this call.
    pub inputs_consumed: usize,
    pub reason: StopReason,
    /// Cumulative instructions retired since construction or snapshot restoration,
    /// excluding incomplete work. Matches [`Machine::retired`].
    pub retired: u64,
}
/// Execution-cost counters. Captures exclude them and restoration resets
/// them; they do not describe guest-visible state.
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
/// In-memory hardware checkpoint, including unfinished operations and emulated
/// time. Host input queues, delivered events and audio history remain caller state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    state: Machine,
}

/// One device with owned hardware state. Run calls require exclusive mutable
/// access; independent instances share no hardware state or callbacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    firmware_origin: [u8; 32],
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
    appointments: Appointments,
    board_appointments: [Option<Time>; 7],
    changed_peripherals: u16,
    changed_board: u8,
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
    #[cfg(feature = "profile-work")]
    work: crate::profile_work::MachineWork,
}
impl Machine {
    /// Start a powered, oscillator-ready device at time zero with default
    /// physical conditions. This starts a new session from persistent images.
    pub fn new(images: Images<'_>) -> Result<Self, Error> {
        Self::with_conditions(images, Conditions::default())
    }
    /// Start a new session with the supplied physical conditions. Zero supply
    /// starts a discharged board; a later supply input can energize it.
    pub fn with_conditions(images: Images<'_>, conditions: Conditions) -> Result<Self, Error> {
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
            mcu,
            eeprom: M95512::new(images.eeprom, images.eeprom_status)?,
            sensor: match images.sensor_nonvolatile {
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
            board_appointments: [None; 7],
            changed_peripherals: 0,
            changed_board: 0,
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
            #[cfg(feature = "profile-work")]
            work: Default::default(),
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
    /// Executor diagnostic. Exact labels may change; use `sleeping` for control.
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
    /// Digital scan selection and polarity at the current observation point.
    /// Includes PWM/FRC and the output latch; excludes drive-voltage amplitudes
    /// and regulator, follower or glass behavior. This does not advance execution.
    pub fn display_drive(&self) -> Result<crate::LcdDrive, Error> {
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
    /// Latched core failure. The session rejects further execution and edits;
    /// restore a healthy checkpoint or construct a new machine to recover.
    /// Rejected input validation and host callback failures do not latch a fault.
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
    /// Capture without advancing execution or emitting events. Diagnostic totals
    /// are excluded. Record the caller's input cursor and output position alongside it.
    pub fn snapshot(&self) -> Snapshot {
        let mut state = self.clone();
        state.cpu.retired = 0;
        state.cpu.interrupt_entries = 0;
        state.stats = Statistics::default();
        #[cfg(feature = "profile-work")]
        state.clear_work();
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
        self.sensor.sync_local_until(self.now, true)?;
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
        let changed = changed | self.changed_peripherals;
        let serial = if changed & schedule::SSU != 0 {
            self.serial_deadline()?
        } else {
            None
        };
        self.appointments.update(changed, &self.mcu, serial)?;
        self.changed_peripherals = 0;
        // Owners report changes to external operations at the actual mutation,
        // such as a sensor register write or EEPROM chip-select release.
        if changed == schedule::ALL {
            self.refresh_board_appointments(0x7f)?;
        } else {
            self.refresh_board_appointments(self.changed_board)?;
        }
        self.changed_board = 0;
        self.next_devices = self
            .board_appointments
            .iter()
            .copied()
            .chain([self.appointments.next()])
            .flatten()
            .min();
        if self.next_devices.is_some_and(|t| t < self.now) {
            return Err(Error::Internal("peripheral appointment in the past"));
        }
        Ok(())
    }
    fn refresh_board_appointments(&mut self, mut dirty: u8) -> Result<(), Error> {
        while dirty != 0 {
            let i = dirty.trailing_zeros() as usize;
            dirty &= dirty - 1;
            self.board_appointments[i] = match i {
                0 => self.power.deadline(),
                1 => self.lcd.deadline(),
                2 => self.mcu.clock_output_deadline(self.now)?,
                3 => self.eeprom.deadline(),
                4 => self.sensor.interaction_deadline(),
                5 => self
                    .watchdog_reset
                    .as_ref()
                    .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?,
                6 => self
                    .reset_release
                    .as_ref()
                    .map_or(Ok(None), |w| w.deadline(&self.mcu.clocks))?,
                _ => unreachable!(),
            };
        }
        Ok(())
    }
    fn serial_data(&self) -> [Option<bool>; 2] {
        let miso = match (self.eeprom.output(), self.sensor.output()) {
            // Opposing external drivers resolve low in the nominal circuit.
            // The electrical basis is in docs/accuracy/bus-and-gpio.md.
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
        self.settle_board(BoardChange::Configuration, out)
    }
    fn settle_board(
        &mut self,
        change: BoardChange,
        out: &mut dyn Output,
    ) -> Result<[bool; 4], Error> {
        #[cfg(feature = "profile-work")]
        self.work.board_settlements.add(1);
        use crate::mcu::gpio::{ALL_PORTS, P1, P3, P8, P9, PB};
        let configuration_changed = matches!(change, BoardChange::Configuration);
        let (owners, clock_output, serial_devices) = match change {
            BoardChange::Configuration => (schedule::ALL, true, true),
            BoardChange::Peripherals {
                owners,
                clock_output,
                serial_devices,
            } => (owners, clock_output, serial_devices),
            BoardChange::Serial => (schedule::SSU, false, true),
        };
        let mut ports = if configuration_changed { ALL_PORTS } else { 0 };
        if clock_output {
            self.mcu.update_clock_output(self.now);
            ports |= P1;
        }
        if owners & schedule::SCI != 0 {
            self.mcu
                .gpio
                .set_sci_pins(self.mcu.sci.pins(), self.incident_light);
            ports |= P3;
        }
        if owners & schedule::AEC != 0 {
            self.mcu
                .gpio
                .set_aec_output(self.mcu.aec.pwm_enabled(), self.mcu.aec.pwm_output());
            ports |= P1;
        }
        if owners & schedule::TIMER_W != 0 {
            ports |= P1 | P8;
        }
        if owners & (schedule::SSU | schedule::IIC) != 0 {
            self.mcu.gpio.set_iic_pins(self.mcu.iic.pins());
            ports |= P9;
        }
        if serial_devices {
            ports |= P9;
        }
        if ports == 0 {
            return Ok(self.mcu.gpio.serial_inputs());
        }
        let previous = self.mcu.gpio.levels;
        let sci_pins = (ports & P3 != 0).then(|| self.mcu.sci.pins());
        let pins = self.mcu.ssu.pins();
        let timer = self.mcu.timer_w.outputs() << 1;
        let timer_mask = self.mcu.timer_w.drives() << 1;
        if self.power.mcu() {
            self.mcu
                .gpio
                .resolve_ports(pins, timer, timer_mask, self.serial_data(), ports & !P9);
        } else {
            self.mcu.gpio.resolve_unpowered(self.serial_data());
        }
        let route = self.mcu.gpio.serial_route(pins);
        let (sampled, mut feedback) =
            self.settle_serial_network(&route, pins, configuration_changed, out)?;
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
        let changed_ports = previous
            .iter()
            .zip(self.mcu.gpio.levels)
            .enumerate()
            .fold(0u8, |mask, (port, (old, new))| {
                mask | (u8::from(*old != new) << port)
            });
        // Selection and an external clock edge can change the SSU's output
        // drivers. Resolve that electrical consequence at this same instant.
        if configuration_changed || changed_ports & P3 != 0 {
            let (sck, rxd) = self.mcu.gpio.sci_inputs();
            if self
                .mcu
                .sci
                .input_pins(sck, rxd, self.now, &self.mcu.clocks)?
            {
                self.changed_peripherals |= schedule::SCI;
            }
        }
        if sci_pins.is_some_and(|old| self.mcu.sci.pins() != old) {
            feedback |= schedule::SCI;
        }
        if feedback != 0 {
            self.settle_board(
                BoardChange::Peripherals {
                    owners: feedback,
                    clock_output: false,
                    serial_devices: false,
                },
                out,
            )?;
        }
        if configuration_changed || changed_ports & P3 != 0 {
            let emitting = self.mcu.gpio.emitting();
            if emitting != self.emitting {
                self.emitting = emitting;
                let _ = out.event(Event::Infrared {
                    at: self.now,
                    emitting,
                });
            }
        }
        if configuration_changed || changed_ports & (P1 | P3 | P9 | PB) != 0 {
            self.mcu.control.pins(self.mcu.gpio.irq_levels());
        }
        if configuration_changed || changed_ports & P8 != 0 {
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
        }
        // Clock edges preserve pin routing. Notify analog and timer inputs
        // when their connected ports change; configuration writes revisit all.
        let connected_ports_changed = configuration_changed
            || changed_ports & (P1 | P3 | P8 | PB) != 0
            || owners & schedule::TIMER_W != 0;
        if !connected_ports_changed {
            return Ok(sampled);
        }
        if self.mcu.comparators.enabled_mask() != 0
            && (configuration_changed || changed_ports & (P3 | PB) != 0)
        {
            self.changed_peripherals |= schedule::COMPARATORS;
            let (pb, vcref) = self.analog_values();
            self.mcu.comparators.set_inputs(
                self.now,
                self.conditions.supply_millivolts,
                vcref,
                [pb[4], pb[5]],
            )?;
        }
        // TIOR can enable capture while the pad remains at its old level.
        // Establish that selected input's baseline before a later edge.
        if (configuration_changed
            || owners & schedule::TIMER_W != 0
            || changed_ports & (P1 | P8) != 0)
            && self.mcu.timer_w.input_pins(
                self.mcu.gpio.timer_inputs(),
                self.now,
                &self.mcu.clocks,
            )?
        {
            self.changed_peripherals |= schedule::TIMER_W;
        }
        if (configuration_changed || changed_ports & P1 != 0)
            && self
                .mcu
                .aec
                .input_pins(self.mcu.gpio.aec_inputs(), self.now, &self.mcu.clocks)?
        {
            self.changed_peripherals |= schedule::AEC;
        }
        if configuration_changed || changed_ports & P1 != 0 {
            self.mcu.collect_aec_requests();
        }
        if configuration_changed {
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
        self.reset_cpu();
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
    fn reset_cpu(&mut self) {
        #[cfg(feature = "profile-work")]
        self.work
            .cpu_phase_carry
            .add(self.cpu.phase_dispatches.get());
        let retired = self.cpu.retired;
        let interrupt_entries = self.cpu.interrupt_entries;
        self.cpu = Cpu::reset();
        self.cpu.retired = retired;
        self.cpu.interrupt_entries = interrupt_entries;
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
        self.sensor.sync_local_until(self.now, false)?;
        self.last_effect = self.now;
        self.stats.peripheral_boundaries = self.stats.peripheral_boundaries.wrapping_add(1);
        let mut due = self.appointments.due(self.now);
        let serial_drivers = self.serial_data();
        let board_due = self
            .board_appointments
            .iter()
            .enumerate()
            .fold(0, |mask, (i, at)| {
                mask | (u8::from(*at == Some(self.now)) << i)
            });
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
                self.reset_cpu();
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
        if self.sensor.interaction_deadline() == Some(self.now) {
            self.sensor.at_deadline(self.now, out)?;
        } else {
            self.sensor.sync_local_until(self.now, true)?;
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
            // TEST shares the ADTRG package input. Only user-mode reset
            // executes the supplied flash image (manual §6.3, Table 6.1).
            if self.mcu.gpio.adc_trigger().1 || !self.mcu.control.nmi_level() {
                return Err(Error::UnsupportedResetMode);
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
        let change = if due == schedule::ALL {
            BoardChange::Configuration
        } else {
            BoardChange::Peripherals {
                owners: due,
                clock_output: board_due & (1 << 2) != 0,
                serial_devices: self.serial_data() != serial_drivers,
            }
        };
        if self.mcu.ssu.deadline(&self.mcu.clocks)? == Some(self.now) {
            self.serial_edge(change, out)?;
            due |= schedule::SSU;
        } else {
            self.settle_board(change, out)?;
        }
        self.refresh_board_appointments(board_due)?;
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
        self.sensor.sync_local_until(self.now, true)?;
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

#[cfg(test)]
mod sensor_sync_tests {
    use super::*;

    #[test]
    fn returned_horizons_and_input_changes_preserve_sensor_apertures() {
        let mut rom = vec![0; 49152];
        rom[..2].copy_from_slice(&[1, 0]);
        rom[0x100..0x110].copy_from_slice(&[
            0x07, 0x80, 0xf8, 0x10, 0x6a, 0x88, 0xff, 0xb1, 0xf8, 0, 0x6a, 0x88, 0xff, 0xb1, 0x40,
            0xfe,
        ]);
        let mut m = Machine::new(Images {
            firmware: &rom,
            eeprom: &[255; 65536],
            eeprom_status: 0,
            sensor_nonvolatile: None,
        })
        .unwrap();
        // The 3-ms readiness interval ends with T/X/Y/Z at 12 kHz.
        let epoch = Time::from_micros(3000).raw() - (1u128 << 64) / 3000;
        let phase = |n: u128| Time::from_raw(epoch + (n << 64) / 12000);
        let temperature = TimedInput {
            at: phase(1),
            input: Input::TemperatureMillicelsius(50_000),
        };
        m.run_until(Time::from_raw(phase(1).raw() + 1), &[temperature], &mut ())
            .unwrap();
        assert_eq!(m.sensor.peek(8), Some(100)); // old 20 °C at the tied aperture
        m.run_until(phase(4), &[], &mut ()).unwrap();
        assert_eq!(m.sensor.peek(7), Some(0)); // Z at the horizon is pending
        let saved = m.snapshot().encode().unwrap();
        let snapshot = Snapshot::decode(&saved).unwrap();
        let mut restored = Machine::from_snapshot(&snapshot);
        let end = Time::from_raw(phase(5).raw() + 1);
        m.run_until(end, &[], &mut ()).unwrap();
        restored.run_until(end, &[], &mut ()).unwrap();
        assert_eq!(m.sensor.peek(7), Some(32)); // +1 g at the default ±4 g range
        assert_eq!(m.sensor.peek(8), Some(160)); // next T converts 50 °C
        assert_eq!(
            m.snapshot().encode().unwrap(),
            restored.snapshot().encode().unwrap()
        );
    }
}
