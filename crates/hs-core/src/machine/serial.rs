//! Synchronize serial shifts at their first possible interaction with other
//! hardware. Register accesses and run horizons materialize any quiet prefix.
use super::*;

impl Machine {
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
        let edge = self.mcu.ssu.advance(self.now, &self.mcu.clocks)?;
        let sampled = self.settle_board(change, out)?;
        if let Some(edge) = edge {
            if edge.sample {
                self.mcu.ssu.sample(sampled[self.mcu.ssu.input_pin()]);
            }
            let pins = self.mcu.ssu.pins();
            self.mcu.ssu.finish_edge(self.now, &self.mcu.clocks)?;
            if self.mcu.ssu.pins() != pins {
                self.settle_board(change, out)?;
            }
        }
        Ok(())
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
        while let Some(at) = self
            .mcu
            .ssu
            .deadline(&self.mcu.clocks)?
            .filter(|at| *at < end)
        {
            self.now = at;
            self.last_effect = self.last_effect.max(at);
            self.stats.peripheral_boundaries = self.stats.peripheral_boundaries.wrapping_add(1);
            let result = self.serial_edge(BoardChange::Serial, out);
            self.now = now;
            result?;
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
