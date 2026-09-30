//! Diagnostic work frequencies. These are host costs, not hardware state or time.
use core::cell::Cell;

/// Cumulative work since construction or restoration, with `profile-work` enabled.
/// Use ordinary builds for timing: updating these totals changes host execution cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Canonical CPU physical-phase dispatches, including projected requests.
    pub cpu_phase_dispatches: u64,
    /// MCU execution views constructed, including immediately rejected operations.
    pub interval_entries: u64,
    pub interval_exits: IntervalExits,
    /// Calls to synchronize MCU owners, including calls selecting no serviced owner.
    pub owner_sync_calls: u64,
    /// MCU owners selected for service while supplied. Serial owners are separate.
    pub owner_syncs: u64,
    /// General board propagation passes.
    pub board_settlements: u64,
    /// Electrical resolution passes over selected board ports.
    pub board_resolutions: u64,
    /// Scalar or packed electrical resolution passes over the serial network.
    pub serial_resolutions: u64,
    /// Physical serial states represented by those resolutions: one per scalar pass.
    pub serial_resolution_lanes: u64,
    /// Scalar SSU edge deliveries through the canonical shifter and network.
    /// This counts delivery work, not clock edges inferred from elapsed time.
    pub serial_edge_deliveries: u64,
    /// Bounded groups of quiet serial halfedges processed together.
    pub serial_prefixes: u64,
    /// Physical halfedges represented by those groups.
    pub serial_prefix_edges: u64,
    /// Temperature/axis aperture phases executed by the BMA150 recurrence.
    pub sensor_sample_phases: u64,
    /// CPU cursor/interval conversions from clock ordinals to timestamps.
    /// Other devices' rational clock projections are not included.
    pub cpu_time_materializations: u64,
}

/// Why CPU intervals yielded to the machine coordinator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntervalExits {
    /// An operation that the coordinator must perform.
    pub request: u64,
    /// An already completed owner operation with consequences to propagate.
    pub committed_owner: u64,
    pub horizon: u64,
    pub exception: u64,
    pub sleep: u64,
    pub reset: u64,
    pub error: u64,
}

impl Work {
    /// Work after an earlier measurement in the same construction/restoration epoch.
    /// Restoring a snapshot starts a new epoch; totals from before it cannot be subtracted.
    pub fn since(self, earlier: Self) -> Self {
        Self {
            cpu_phase_dispatches: self
                .cpu_phase_dispatches
                .wrapping_sub(earlier.cpu_phase_dispatches),
            interval_entries: self.interval_entries.wrapping_sub(earlier.interval_entries),
            owner_sync_calls: self.owner_sync_calls.wrapping_sub(earlier.owner_sync_calls),
            owner_syncs: self.owner_syncs.wrapping_sub(earlier.owner_syncs),
            board_settlements: self
                .board_settlements
                .wrapping_sub(earlier.board_settlements),
            board_resolutions: self
                .board_resolutions
                .wrapping_sub(earlier.board_resolutions),
            serial_resolutions: self
                .serial_resolutions
                .wrapping_sub(earlier.serial_resolutions),
            serial_resolution_lanes: self
                .serial_resolution_lanes
                .wrapping_sub(earlier.serial_resolution_lanes),
            serial_edge_deliveries: self
                .serial_edge_deliveries
                .wrapping_sub(earlier.serial_edge_deliveries),
            serial_prefixes: self.serial_prefixes.wrapping_sub(earlier.serial_prefixes),
            serial_prefix_edges: self
                .serial_prefix_edges
                .wrapping_sub(earlier.serial_prefix_edges),
            sensor_sample_phases: self
                .sensor_sample_phases
                .wrapping_sub(earlier.sensor_sample_phases),
            cpu_time_materializations: self
                .cpu_time_materializations
                .wrapping_sub(earlier.cpu_time_materializations),
            interval_exits: IntervalExits {
                request: self
                    .interval_exits
                    .request
                    .wrapping_sub(earlier.interval_exits.request),
                committed_owner: self
                    .interval_exits
                    .committed_owner
                    .wrapping_sub(earlier.interval_exits.committed_owner),
                horizon: self
                    .interval_exits
                    .horizon
                    .wrapping_sub(earlier.interval_exits.horizon),
                exception: self
                    .interval_exits
                    .exception
                    .wrapping_sub(earlier.interval_exits.exception),
                sleep: self
                    .interval_exits
                    .sleep
                    .wrapping_sub(earlier.interval_exits.sleep),
                reset: self
                    .interval_exits
                    .reset
                    .wrapping_sub(earlier.interval_exits.reset),
                error: self
                    .interval_exits
                    .error
                    .wrapping_sub(earlier.interval_exits.error),
            },
        }
    }
}
/// Each component owns its counters. Cloning a component copies its totals;
/// inspecting or materializing the clone cannot alter the running instance.
#[derive(Clone, Debug, Default)]
pub(crate) struct Counter(Cell<u64>);
impl Counter {
    pub fn add(&self, n: u64) {
        self.0.set(self.0.get().wrapping_add(n));
    }
    pub fn get(&self) -> u64 {
        self.0.get()
    }
    pub fn clear(&self) {
        self.0.set(0);
    }
}
// Work grouping changes frequencies without changing causal hardware state.
// Reports above compare actual totals; component equality deliberately ignores them.
impl PartialEq for Counter {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Counter {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MachineWork {
    pub cpu_phase_carry: Counter,
    pub interval_entries: Counter,
    pub request: Counter,
    pub committed_owner: Counter,
    pub horizon: Counter,
    pub exception: Counter,
    pub sleep: Counter,
    pub reset: Counter,
    pub error: Counter,
    pub board_settlements: Counter,
    pub serial_edge_deliveries: Counter,
    pub serial_prefixes: Counter,
    pub serial_prefix_edges: Counter,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnerWork {
    pub calls: Counter,
    pub owners: Counter,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResolutionWork {
    pub board: Counter,
    pub serial: Counter,
    pub serial_lanes: Counter,
}
