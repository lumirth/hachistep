//! Synchronize serial shifts at their first possible interaction with other
//! hardware. Register accesses and run horizons materialize any quiet prefix.
use super::*;
use crate::mcu::{gpio::SerialRoute, ssu::Pins};

impl Machine {
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
        let iic_pins = self.mcu.iic.pins();
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
        if configuration_changed || previous != self.mcu.gpio.levels[3] {
            let serial = route.inputs(self.mcu.gpio.levels[3]);
            if let Some(edge) = self.mcu.ssu.input_pins(serial[0], serial[1]) {
                #[cfg(feature = "profile-work")]
                self.work.serial_edge_deliveries.add(1);
                self.changed_peripherals |= schedule::SSU;
                if edge.sample {
                    self.mcu.ssu.sample(sampled[self.mcu.ssu.input_pin()]);
                }
                self.mcu.ssu.finish_edge(self.now, &self.mcu.clocks)?;
            }
        }
        if configuration_changed || previous != self.mcu.gpio.levels[3] {
            let [scl, sda] = self.mcu.gpio.iic_inputs();
            if self
                .mcu
                .iic
                .input_pins(scl, sda, self.now, &self.mcu.clocks)?
            {
                self.changed_peripherals |= schedule::IIC;
            }
        }
        let mut feedback = 0;
        if previous != self.mcu.gpio.levels[3] || configuration_changed {
            if self.mcu.ssu.pins() != pins {
                feedback |= schedule::SSU;
            }
            if self.mcu.iic.pins() != iic_pins {
                feedback |= schedule::IIC;
            }
        }
        if !configuration_changed && previous != self.mcu.gpio.levels[3] && route.irq_observer {
            self.mcu.control.pins(self.mcu.gpio.irq_levels());
        }
        Ok((sampled, feedback))
    }

    pub(super) fn serial_deadline(&self) -> Result<Option<Time>, Error> {
        let edges = if self.mcu.gpio.pfcr & 0x10 != 0
            || self.mcu.gpio.irq_routes().contains(&2)
            || self.mcu.iic.pins().is_some()
        {
            1
        } else {
            self.lcd
                .serial_effect_edges()
                .min(self.eeprom.serial_effect_edges())
                .min(self.sensor.serial_effect_edges())
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
        let was_master = self.mcu.ssu.master();
        let edge = self.mcu.ssu.advance(self.now, &self.mcu.clocks)?;
        #[cfg(feature = "profile-work")]
        if edge.is_some() {
            self.work.serial_edge_deliveries.add(1);
        }
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
        Ok(feedback || ownership_changed)
    }
    // CPU memory accesses can pass quiet shifts. Materialize those shifts
    // before an observer or another device can interact with the serial bus.
    pub(super) fn sync_serial_before(
        &mut self,
        end: Time,
        out: &mut dyn Output,
    ) -> Result<bool, Error> {
        let now = self.now;
        let mut changed = false;
        let mut route = self.mcu.gpio.serial_route(self.mcu.ssu.pins());
        while let Some(at) = self
            .mcu
            .ssu
            .deadline(&self.mcu.clocks)?
            .filter(|at| *at < end)
        {
            self.now = at;
            self.last_effect = self.last_effect.max(at);
            self.stats.peripheral_boundaries = self.stats.peripheral_boundaries.wrapping_add(1);
            let result = self.serial_edge_with_route(BoardChange::Serial, Some(&route), out);
            self.now = now;
            if result? {
                route = self.mcu.gpio.serial_route(self.mcu.ssu.pins());
            }
            changed = true;
        }
        if changed {
            self.refresh_peripherals(schedule::SSU)?;
        }
        Ok(changed)
    }
    pub(super) fn sync_serial(&mut self, out: &mut dyn Output) -> Result<bool, Error> {
        self.sync_serial_before(Time::from_raw(self.now.raw() + 1), out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cpu::WriteOrigin, signals::DigitalPin};

    #[test]
    fn a_master_load_conflict_rebuilds_slave_driver_ownership_before_resolution() {
        let mut firmware = vec![0; 49152];
        firmware[..2].copy_from_slice(&0x100u16.to_be_bytes());
        firmware[0x100..0x102].copy_from_slice(&[0x40, 0xfe]);
        let mut m = Machine::new(Images {
            firmware: &firmware,
            eeprom: &[255; 65536],
            eeprom_status: 0,
            sensor_nonvolatile: None,
        })
        .unwrap();
        for (address, value) in [
            (0xfffb, 0x14),
            (0xf0e0, 0x8e),
            (0xf0e1, 0x40),
            (0xf0e2, 0x86),
            (0xf0e3, 0x80),
            (0xf087, 6),
            (0xffec, 8),
            (0xffdc, 8),
        ] {
            m.mcu
                .write8(address, value, WriteOrigin::MovByte, m.now, &mut ())
                .unwrap();
        }
        // An already-low SCS conflicts with a new four-line master load. In
        // slave mode, transmit moves from SSO/P92 to SSI/P93. The selected
        // peripheral overrides P93's high GPIO latch with its low SOL driver.
        m.mcu.gpio.set_digital_level(DigitalPin::P90, Some(false));
        m.resolve_board(&mut ()).unwrap();
        assert_eq!(m.mcu.gpio.levels[3] & 8, 8);
        m.mcu
            .ssu
            .write(0xf0eb, 0xa5, true, m.now, &m.mcu.clocks)
            .unwrap();
        m.now = m.mcu.ssu.deadline(&m.mcu.clocks).unwrap().unwrap();
        m.sync_serial(&mut ()).unwrap();
        assert_eq!(m.mcu.ssu.peek(0xf0e4) & 1, 1, "CE becomes asserted");
        assert_eq!(m.mcu.ssu.peek(0xf0e0) & 0x80, 0, "master mode is released");
        assert_eq!(
            m.mcu.gpio.levels[3] & 12,
            4,
            "P92 pulls high; SSI drives P93 low"
        );
        assert_eq!(
            m.mcu.gpio.read(0xffdc),
            14,
            "output readback still uses the P93 latch"
        );
        m.refresh_peripherals(schedule::ALL).unwrap();
        let saved = m.snapshot().encode().unwrap();
        let restored = Machine::from_snapshot(&Snapshot::decode(&saved).unwrap());
        assert_eq!(restored.mcu.gpio.levels[3] & 12, 4);
        assert_eq!(restored.snapshot().encode().unwrap(), saved);
    }
}
