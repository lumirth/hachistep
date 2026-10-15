//! Synchronize serial shifts at their first possible interaction with other
//! hardware. Register accesses and run horizons materialize any quiet prefix.
use super::*;
use crate::devices::{bma150::Bma150, m95512::M95512, nt7508::Nt7508};
use crate::mcu::{
    clocks::{Clocks, CpuWindow},
    gpio::{Gpio, SerialLevels, SerialRoute},
    iic::Iic,
    ssu::{Instant, Pins, Ssu},
};
use crate::power::Power;

/// Port 9 levels indexed by shift clock (bit 0), data output (bit 1) and
/// the EEPROM's answer (low, high, released: bits 2-3).
#[derive(Clone, Copy, Debug)]
pub(super) struct ShiftNet {
    levels: [u8; 12],
    /// The bits carrying the SSU's sampled input, select and clock inputs.
    sample: u8,
    select: u8,
    clock: u8,
    /// Direct MOSI and EEPROM-to-SSI wiring; the value sampled on release.
    paired_input_float: Option<bool>,
}
/// The shift net of the last configuration it was built for. It is derived
/// from the board state named by its key, so it is not state of its own.
#[derive(Clone, Debug, Default)]
pub(super) struct ShiftCache {
    key: [u8; 14],
    net: Option<Option<ShiftNet>>,
    /// Plain events were performed since the board last looked.
    progressed: bool,
}
impl PartialEq for ShiftCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for ShiftCache {}

impl Board {
    /// Settle the fixed serial nets and their connected consumers. Both master
    /// runs and ordinary board changes enter this one electrical mechanism.
    pub(super) fn settle_serial_network(
        &mut self,
        route: &SerialRoute,
        pins: Pins,
        configuration_changed: bool,
        out: &mut dyn Output,
    ) -> Result<([bool; 4], u16), Error> {
        let previous = self.mcu.gpio.levels[3];
        let data = self.serial_data();
        let levels = if self.power.mcu() {
            self.mcu.gpio.resolve_serial_route(route, pins, data)
        } else {
            self.mcu.gpio.resolve_unpowered(data)
        };
        let sampled = route.inputs(self.mcu.gpio.levels[3]);
        if self.power.eeprom()
            && (configuration_changed || levels.eeprom_selected != self.serial.eeprom_selected)
            && self
                .eeprom
                .select_effect(levels.eeprom_selected, self.now)?
        {
            self.changed_board |= 1 << 3;
        }
        if configuration_changed || levels.sensor_selected != self.serial.sensor_selected {
            self.sensor.set_selected(levels.sensor_selected);
        }
        if !self.serial.sensor_selected
            && !levels.sensor_selected
            && (self.serial.clock != levels.clock || self.serial.mosi != levels.mosi)
            && self.sensor.i2c_pins_at(
                [self.serial.clock, self.serial.mosi],
                [levels.clock, levels.mosi],
                self.now,
            )?
        {
            self.changed_board |= 1 << 4;
        }
        if configuration_changed || levels.lcd_selected != self.serial.lcd_selected {
            self.lcd.select(levels.lcd_selected);
        }
        if configuration_changed || levels.data != self.serial.data {
            self.lcd.command_data(levels.data);
        }
        if levels.clock != self.serial.clock {
            if levels.clock {
                if self.power.eeprom() {
                    self.eeprom.rising(levels.mosi);
                }
                if self.sensor.rising_at(levels.mosi, self.now)? {
                    self.changed_board |= 1 << 4;
                }
                self.lcd.rising(levels.mosi, self.now, out)?;
            } else {
                self.eeprom.falling();
                self.sensor.falling_at(self.now)?;
            }
        }
        let settled_data = self.serial_data();
        // Only the external serial drivers can have changed since the first
        // resolution. Preserve it when the electrical inputs are identical.
        self.serial = if settled_data == data {
            levels
        } else if self.power.mcu() {
            self.mcu
                .gpio
                .resolve_serial_route(route, pins, settled_data)
        } else {
            self.mcu.gpio.resolve_unpowered(settled_data)
        };
        if !self.power.mcu() {
            return Ok((sampled, 0));
        }
        if !configuration_changed && previous == self.mcu.gpio.levels[3] {
            return Ok((sampled, 0));
        }
        // Devices change no MCU state, so the SSU and IIC are still in the
        // state `pins` was derived from; it can only differ later if the
        // state it depends on changes.
        let pin_state = self.mcu.ssu.pin_state();
        let iic_pins = self.mcu.iic.pins();
        let serial = route.inputs(self.mcu.gpio.levels[3]);
        if let Some(edge) = self.mcu.ssu.input_pins(serial[0], serial[1]) {
            self.changed_peripherals |= schedule::SSU;
            if edge.sample {
                self.mcu.ssu.sample(sampled[self.mcu.ssu.input_pin()]);
            }
            self.mcu.ssu.finish_edge(self.now, &self.mcu.clocks)?;
        }
        let [scl, sda] = self.mcu.gpio.iic_inputs();
        if self
            .mcu
            .iic
            .input_pins(scl, sda, self.now, &self.mcu.clocks)?
        {
            self.changed_peripherals |= schedule::IIC;
        }
        let mut feedback = 0;
        if self.mcu.ssu.pin_state() != pin_state && self.mcu.ssu.pins() != pins {
            feedback |= schedule::SSU;
        }
        if self.mcu.iic.pins() != iic_pins {
            feedback |= schedule::IIC;
        }
        if !configuration_changed && route.irq_observer {
            self.mcu.control.pins(self.mcu.gpio.irq_levels());
        }
        Ok((sampled, feedback))
    }

    pub(super) fn serial_deadline(&mut self) -> Result<Option<Time>, Error> {
        if self.serial_lazy() {
            return Ok(None);
        }
        let edges = if self.mcu.gpio.pfcr & 0x10 != 0
            || self.mcu.gpio.irq_routes().contains(&2)
            || self.mcu.iic.pins().is_some()
        {
            1
        } else {
            // Each shift clock edge moves the clock net once.
            let clock = self.serial.clock;
            self.lcd
                .serial_effect_edges(clock)
                .min(self.eeprom.serial_effect_edges(clock))
                .min(self.sensor.serial_effect_edges(clock))
        };
        self.mcu.ssu.effect_deadline(edges, &self.mcu.clocks)
    }
    pub(super) fn serial_edge(
        &mut self,
        change: BoardChange,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        self.serial_edge_with_route(change, None, out).map(|_| ())
    }
    fn settle_serial_run(
        &mut self,
        route: &SerialRoute,
        out: &mut dyn Output,
    ) -> Result<([bool; 4], bool), Error> {
        let pins = self.mcu.ssu.pins();
        let (sampled, feedback) = self.settle_serial_network(route, pins, false, out)?;
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
        Ok((sampled, feedback != 0))
    }
    fn serial_edge_with_route(
        &mut self,
        change: BoardChange,
        route: Option<&SerialRoute>,
        out: &mut dyn Output,
    ) -> Result<bool, Error> {
        // A raw SSU edge can coincide with another owner's appointment before
        // its batched effect deadline. It still changes the serial drivers.
        let change = match change {
            BoardChange::Peripherals {
                owners,
                clock_output,
                serial_devices,
            } => BoardChange::Peripherals {
                owners: owners | schedule::SSU,
                clock_output,
                serial_devices,
            },
            other => other,
        };
        let plain = match change {
            BoardChange::Serial => true,
            BoardChange::Peripherals {
                owners,
                clock_output,
                serial_devices,
            } => owners == schedule::SSU && !clock_output && !serial_devices,
            BoardChange::Configuration => false,
        };
        let now = self.now;
        if plain
            && self
                .with_wires(out, |wires| wires.run(now, true))?
                .is_none()
        {
            self.changed_peripherals |= schedule::SSU;
            return Ok(false);
        }
        let was_master = self.mcu.ssu.master();
        let edge = self.mcu.ssu.advance(self.now, &self.mcu.clocks)?;
        // A four-line load conflict changes master/slave pin ownership. Rebuild
        // the disposable route before resolving that same physical consequence.
        let ownership_changed = was_master != self.mcu.ssu.master();
        let replacement = (route.is_some() && ownership_changed)
            .then(|| self.mcu.gpio.serial_route(self.mcu.ssu.pins()));
        let route = replacement.as_ref().or(route);
        let (sampled, mut feedback) = if let Some(route) = route {
            self.settle_serial_run(route, out)?
        } else {
            (self.settle_board(change, out)?, false)
        };
        if let Some(edge) = edge {
            self.changed_peripherals |= schedule::SSU;
            if edge.sample {
                self.mcu.ssu.sample(sampled[self.mcu.ssu.input_pin()]);
            }
            if self.mcu.ssu.finishing() {
                let pins = self.mcu.ssu.pins();
                self.mcu.ssu.finish_edge(self.now, &self.mcu.clocks)?;
                if self.mcu.ssu.pins() != pins {
                    if let Some(route) = route {
                        feedback |= self.settle_serial_run(route, out)?.1;
                    } else {
                        self.settle_board(change, out)?;
                    }
                }
            }
        }
        Ok(feedback || ownership_changed)
    }
    /// Borrow the serial engine without moving its device-reference bundle.
    pub(super) fn with_wires<R>(
        &mut self,
        out: &mut dyn Output,
        f: impl FnOnce(&mut Wires<'_, '_>) -> R,
    ) -> R {
        let (ssu, gpio, iic, clocks) = self.mcu.serial_parts();
        let mut devices = SerialDevices {
            lcd: &mut self.lcd,
            eeprom: &mut self.eeprom,
            sensor: &self.sensor,
            power: &self.power,
            serial: &mut self.serial,
            shift: &mut self.shift,
            edges: &mut self.stats.peripheral_boundaries,
            last_effect: &mut self.last_effect,
            late: self.late_output.0,
        };
        f(&mut Wires {
            ssu,
            gpio,
            iic,
            clocks,
            devices: &mut devices,
            out,
            now: self.now,
        })
    }
    /// Whether a register the access reads changes only at its owners'
    /// appointments or by accesses (`Access::settled`): lazy SSU and SCI
    /// flags change without appointments.
    pub(super) fn settled(&mut self, access: crate::mcu::bus::Access) -> bool {
        access.settled()
            && !(access.serial_engine() && self.serial_lazy())
            && !(access.owners & schedule::SCI != 0 && self.mcu.sci.lazy(0))
    }
    /// Whether SSU activity needs no appointment (`Wires::lazy`).
    pub(super) fn serial_lazy(&mut self) -> bool {
        let mut out = ();
        self.with_wires(&mut out, |wires| wires.lazy())
    }
    // CPU memory accesses can pass serial shifts. Materialize those shifts
    // before an observer or another device can interact with the serial bus.
    pub(super) fn sync_serial_before(
        &mut self,
        end: Time,
        out: &mut dyn Output,
    ) -> Result<bool, Error> {
        self.report_optical_before(end, out)?;
        let now = self.now;
        let mut changed = false;
        let mut route = None;
        loop {
            let (stop, progressed) = self.with_wires(out, |wires| {
                let stop = wires.run(end, false);
                (stop, wires.progressed())
            });
            changed |= progressed;
            let Some(at) = stop? else {
                break;
            };
            let route =
                route.get_or_insert_with(|| self.mcu.gpio.serial_route(self.mcu.ssu.pins()));
            self.now = at;
            self.last_effect = self.last_effect.max(at);
            self.stats.peripheral_boundaries = self.stats.peripheral_boundaries.wrapping_add(1);
            let result = self.serial_edge_with_route(BoardChange::Serial, Some(route), out);
            self.now = now;
            if result? {
                *route = self.mcu.gpio.serial_route(self.mcu.ssu.pins());
            }
            changed = true;
        }
        if changed {
            self.changed_peripherals |= schedule::SSU;
            self.refresh_peripherals(schedule::SSU)?;
        }
        Ok(changed)
    }
    pub(super) fn sync_serial(&mut self, out: &mut dyn Output) -> Result<bool, Error> {
        self.sync_serial_before(Time::from_raw(self.now.raw() + 1), out)
    }
}

/// The external serial devices and the board's record of the serial nets.
pub(super) struct SerialDevices<'a> {
    pub lcd: &'a mut Nt7508,
    pub eeprom: &'a mut M95512,
    pub sensor: &'a Bma150,
    pub power: &'a Power,
    pub serial: &'a mut SerialLevels,
    pub shift: &'a mut ShiftCache,
    /// Peripheral boundaries performed (`Statistics`).
    pub edges: &'a mut u64,
    pub last_effect: &'a mut Time,
    /// The run's sink accepts late output (`Output::exact_stops`).
    pub late: bool,
}

/// The serial engine: the SSU, port 9 and the devices on its nets, with
/// what they observe. It performs plain SSU events, those whose every
/// consequence stays among these parts and the output: the shift edges of a
/// master frame on its shift net (`Wires::shift_net`), and loads and frame
/// completions that move no pin. Anything else is left to the board.
///
/// The parts it touches are observable only through the serial bus, the SSU
/// registers and the output, so plain events can be performed late, at
/// their own times, whenever one of those is next used, in order before it.
pub(super) struct Wires<'a, 'd> {
    pub ssu: &'a mut Ssu,
    pub gpio: &'a mut Gpio,
    pub iic: &'a mut Iic,
    pub clocks: &'a Clocks,
    pub devices: &'a mut SerialDevices<'d>,
    pub out: &'a mut dyn Output,
    /// The time of the event being performed.
    pub now: Time,
}
impl Wires<'_, '_> {
    /// Whether every SSU event can wait for the next use of the serial bus,
    /// the SSU registers or the output: they are plain and request no
    /// interrupt, and the sink accepts late output. Then the SSU needs no
    /// appointment.
    pub(super) fn lazy(&mut self) -> bool {
        self.lazy_net().is_some()
    }
    /// The shift net, when the engine is lazy.
    pub(super) fn lazy_net(&mut self) -> Option<ShiftNet> {
        if !self.devices.late || self.ssu.interrupts_enabled() || !self.ssu.plain_frames() {
            return None;
        }
        self.shift_net()
    }
    /// Whether plain events were performed since the last call.
    fn progressed(&mut self) -> bool {
        std::mem::take(&mut self.devices.shift.progressed)
    }
    /// The levels port 9 can take during the running master frame, when its
    /// edges are plain shifts: the SSU drives the shift clock straight onto
    /// its net, the select nets hold, and the only external driver that can
    /// answer is the EEPROM. None when an edge can reach anything else.
    fn shift_net(&mut self) -> Option<ShiftNet> {
        let devices = &self.devices;
        if !devices.power.mcu() || self.iic.observes_pins() || !devices.sensor.serially_idle() {
            return None;
        }
        let mut key = [0; 14];
        key[..8].copy_from_slice(&self.gpio.serial_key());
        key[8..13].copy_from_slice(&self.ssu.pin_key());
        key[13] = devices.sensor.data_output() as u8;
        if devices.shift.key != key || devices.shift.net.is_none() {
            let net = self.build_shift_net();
            self.devices.shift.key = key;
            self.devices.shift.net = Some(net);
        }
        let net = self.devices.shift.net.flatten()?;
        // The select net is not a shift output, so it holds for the frame.
        let select_high = self.gpio.levels[3] & net.select != 0;
        self.ssu.sees_select(select_high).then_some(net)
    }
    fn build_shift_net(&self) -> Option<ShiftNet> {
        let pins = self.ssu.pins();
        let route = self.gpio.serial_route(pins);
        if route.irq_observer || !route.follows(1, pins) {
            return None;
        }
        // The deselected idle sensor drives no MISO and at most a constant
        // SDA (`Bma150::serially_idle`).
        let mosi = match self.devices.sensor.data_output() {
            Drive::Low => Some(false),
            Drive::High => Some(true),
            Drive::Floating => None,
        };
        let mut levels = [0; 12];
        for (index, level) in levels.iter_mut().enumerate() {
            let pins = self.ssu.shift_pins_at(pins, index & 1 != 0, index & 2 != 0);
            *level = route.levels(pins, [mosi, [Some(false), Some(true), None][index >> 2]]);
        }
        let sample = route.input_mask(self.ssu.input_pin());
        let select = route.input_mask(0);
        let floating = levels[8] & sample != 0;
        let byte_wired = levels.iter().enumerate().all(|(i, &level)| {
            (level & 2 != 0) == (i & 1 != 0)
                && (level & 4 != 0) == (i & 2 != 0)
                && (level & select) == (levels[0] & select)
                && (level & sample != 0)
                    == match i >> 2 {
                        0 => false,
                        1 => true,
                        _ => floating,
                    }
        });
        Some(ShiftNet {
            levels,
            sample,
            select,
            clock: route.input_mask(1),
            paired_input_float: byte_wired.then_some(floating),
        })
    }
    /// Port 9 under the SSU's current shift outputs and the EEPROM's answer.
    #[inline]
    fn shift_levels(&self, net: &ShiftNet) -> u8 {
        let miso = match self.devices.eeprom.output() {
            Drive::Low => 0,
            Drive::High => 4,
            Drive::Floating => 8,
        };
        net.levels[self.ssu.shift_levels() | miso]
    }
    /// After plain events: the SSU and IIC record the levels they last saw.
    /// Neither responds to them (`shift_net`), so the last levels stand for
    /// every edge's.
    fn shift_settled(&mut self, net: &ShiftNet) -> Result<(), Error> {
        let levels = self.gpio.levels[3];
        if self
            .ssu
            .input_pins(levels & net.select != 0, levels & net.clock != 0)
            .is_some()
        {
            return Err(Error::Internal("master SSU accepted an external edge"));
        }
        let [scl, sda] = self.gpio.iic_inputs();
        self.iic.input_pins(scl, sda, self.now, self.clocks)?;
        Ok(())
    }
    /// Perform the plain SSU events before `end`, or through it when
    /// `inclusive`. Returns the time of the first other event in that range,
    /// left for the board, or None.
    pub(super) fn run(&mut self, end: Time, inclusive: bool) -> Result<Option<Time>, Error> {
        let limit = if inclusive {
            Time::from_raw(end.raw() + 1)
        } else {
            end
        };
        if self.ssu.deadline(self.clocks)?.is_none_or(|at| at >= limit) {
            return Ok(None);
        }
        match self.shift_net() {
            Some(net) => self.run_on(&net, limit, None),
            None => self.ssu.deadline(self.clocks),
        }
    }
    /// `run` on `net`, the shift net, for events before `limit`. Plain events
    /// leave the net as it is. `through`, when given, supplies the CPU window and
    /// ordinal of the last edge before `limit`, which lets clock waits be
    /// placed by ordinal (`ssu::Instant`).
    pub(super) fn run_on(
        &mut self,
        net: &ShiftNet,
        limit: Time,
        through: Option<(&CpuWindow, u64)>,
    ) -> Result<Option<Time>, Error> {
        loop {
            let Some(at) = self.ssu.deadline(self.clocks)?.filter(|at| *at < limit) else {
                return Ok(None);
            };
            if self.ssu.shifting() {
                // A four-line select output can move as the frame completes.
                let edges = if self.ssu.plain_frames() {
                    self.ssu.frame_edges()
                } else {
                    self.ssu.frame_edges() - 1
                };
                if edges == 0 {
                    return Ok(Some(at));
                }
                self.shift_edges(net, at, limit, through, edges)?;
            } else if self.ssu.loads_plainly() {
                self.now = at;
                let instant = Instant {
                    time: at,
                    cpu: through.and(self.ssu.cpu_edge(0, self.clocks)),
                };
                if self.ssu.advance_at(instant, self.clocks)?.is_some() {
                    return Err(Error::Internal("SSU load with an edge"));
                }
                // At most the data output moved, with the shift clock low.
                let levels = self.shift_levels(net);
                self.gpio.levels[3] = levels;
                self.devices.serial.mosi = levels & 4 != 0;
                self.shift_settled(net)?;
                self.performed(at, 1);
            } else {
                return Ok(Some(at));
            }
        }
    }
    fn performed(&mut self, last: Time, events: u64) {
        let devices = &mut self.devices;
        *devices.last_effect = (*devices.last_effect).max(last);
        *devices.edges = devices.edges.wrapping_add(events);
        devices.shift.progressed = true;
    }
    /// Up to `most` edges of the running frame, from the one at `first`,
    /// before `limit`.
    fn shift_edges(
        &mut self,
        net: &ShiftNet,
        first: Time,
        limit: Time,
        through: Option<(&CpuWindow, u64)>,
        most: u64,
    ) -> Result<(), Error> {
        // Aligned clock ordinals count a partial frame as well as a complete
        // one, without converting the timestamp back through the prescaler.
        let cpu_run = through.and_then(|_| self.ssu.cpu_run(self.clocks));
        let ordinal = |count: u64| {
            let (first, period) = cpu_run?;
            first.checked_add(count.checked_mul(period)?)
        };
        let edge_time = |count: u64| match (through, ordinal(count)) {
            (Some((window, _)), Some(edge)) => window.time_of_ordinal(edge).map_or_else(
                || self.ssu.later_edge(count, self.clocks),
                |at| at.map(Some),
            ),
            _ => self.ssu.later_edge(count, self.clocks),
        };
        let projected = through.and_then(|(_, through)| {
            let (first, period) = cpu_run?;
            Some(
                (through.checked_sub(first)? / period)
                    .saturating_add(1)
                    .min(most),
            )
        });
        let (edges, last, last_cpu) = match projected {
            Some(edges) => {
                let last =
                    edge_time(edges - 1)?.ok_or(Error::Internal("SSU edge outside its range"))?;
                (edges, last, ordinal(edges - 1))
            }
            None => {
                let mut edges = self.ssu.edges_before(limit, self.clocks).clamp(1, most);
                let last = loop {
                    match edge_time(edges - 1)? {
                        Some(at) if at < limit => break at,
                        Some(_) if edges > 1 => edges -= 1,
                        _ => return Err(Error::Internal("SSU edge outside its range")),
                    }
                };
                (edges, last, ordinal(edges - 1))
            }
        };
        // Only the byte the LCD completes uses its edge's time. The EEPROM
        // and sensor shift without time, and the IIC module, which does not
        // observe the pins, records raw levels only.
        let devices = &self.devices;
        let lcd = u64::from(devices.lcd.serial_effect_edges(devices.serial.clock));
        let lcd_at = if lcd < edges {
            edge_time(lcd - 1)?.ok_or(Error::Internal("SSU edge outside its range"))?
        } else {
            last
        };
        // The edges reach the devices in order; the LCD only collects bits
        // until the edge that completes its byte, so it takes them at once.
        let mut lcd_bits = 0u8;
        let mut lcd_count = 0u8;
        let lcd_time = if lcd <= edges { lcd_at } else { first };
        let mut edge = 0;
        while edge < edges {
            let pairs = ((edges - edge) / 2) as u8;
            if pairs != 0 && self.devices.power.eeprom() {
                if let Some((floating, tx)) = net
                    .paired_input_float
                    .and_then(|floating| self.ssu.output_bits(pairs).map(|tx| (floating, tx)))
                {
                    let rx = self.devices.eeprom.exchange_bits(tx, pairs, floating);
                    self.ssu.shifted_bits(rx, pairs);
                    lcd_bits = ((u16::from(lcd_bits) << pairs) | u16::from(tx)) as u8;
                    lcd_count += pairs;
                    self.devices.serial.clock = true;
                    self.devices.serial.mosi = tx & 1 != 0;
                    edge += 2 * u64::from(pairs);
                    continue;
                }
            }
            let shifted = self.ssu.quiet_edge();
            let levels = self.shift_levels(net);
            let clock = levels & 2 != 0;
            let mosi = levels & 4 != 0;
            let devices = &mut self.devices;
            if clock != devices.serial.clock {
                if clock {
                    if devices.power.eeprom() {
                        devices.eeprom.rising(mosi);
                    }
                    lcd_bits = lcd_bits << 1 | u8::from(mosi);
                    lcd_count += 1;
                } else {
                    devices.eeprom.falling();
                }
            }
            devices.serial.clock = clock;
            devices.serial.mosi = mosi;
            if shifted.sample {
                self.ssu.sample(levels & net.sample != 0);
            }
            edge += 1;
        }
        self.gpio.levels[3] = self.shift_levels(net);
        self.now = lcd_time;
        self.devices
            .lcd
            .shift_in(lcd_bits, lcd_count, lcd_time, self.out)?;
        self.now = last;
        self.ssu.shifted(edges, self.clocks)?;
        self.shift_settled(net)?;
        if self.ssu.finishing() {
            let instant = Instant {
                time: last,
                cpu: last_cpu,
            };
            self.ssu.finish_edge_at(instant, self.clocks)?;
        }
        self.performed(last, edges);
        Ok(())
    }
}
