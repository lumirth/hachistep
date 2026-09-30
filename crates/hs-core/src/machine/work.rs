use super::Machine;
use crate::profile_work::{IntervalExits, Work};

impl Machine {
    pub(crate) fn work(&self) -> Work {
        let work = &self.work;
        Work {
            cpu_phase_dispatches: work
                .cpu_phase_carry
                .get()
                .wrapping_add(self.cpu.phase_dispatches.get()),
            interval_entries: work.interval_entries.get(),
            interval_exits: IntervalExits {
                request: work.request.get(),
                committed_owner: work.committed_owner.get(),
                horizon: work.horizon.get(),
                exception: work.exception.get(),
                sleep: work.sleep.get(),
                reset: work.reset.get(),
                error: work.error.get(),
            },
            owner_sync_calls: self.mcu.work.calls.get(),
            owner_syncs: self.mcu.work.owners.get(),
            board_settlements: work.board_settlements.get(),
            board_resolutions: self.mcu.gpio.work.board.get(),
            serial_resolutions: self.mcu.gpio.work.serial.get(),
            serial_resolution_lanes: self.mcu.gpio.work.serial_lanes.get(),
            serial_edge_deliveries: work.serial_edge_deliveries.get(),
            serial_prefixes: work.serial_prefixes.get(),
            serial_prefix_edges: work.serial_prefix_edges.get(),
            sensor_sample_phases: self.sensor.sample_phases.get(),
            cpu_time_materializations: self.mcu.clocks.cpu_time_materializations.get(),
        }
    }

    pub(super) fn clear_work(&mut self) {
        self.work = Default::default();
        self.cpu.phase_dispatches.clear();
        self.mcu.work = Default::default();
        self.mcu.gpio.work = Default::default();
        self.mcu.clocks.cpu_time_materializations.clear();
        self.sensor.sample_phases.clear();
    }
}
