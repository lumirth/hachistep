#![cfg(feature = "profile-work")]

use hs_core::{
    diagnostic, Conditions, Frequencies, Images, Input, Machine, Snapshot, Time, TimedInput,
};

fn machine(looping: bool) -> Machine {
    let mut code = Vec::new();
    for (address, value) in [(0xffb1u16, 0x10), (0xffb1, 0)] {
        code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
    }
    code.extend([0xf8, 0x2a, 0x6a, 0x88, 0xf7, 0x80]);
    if looping {
        // Store R0L, increment it and branch to the store.
        code.extend([0x0a, 0x08, 0x40, 0xf8]);
    } else {
        code.extend([0x01, 0x80, 0x40, 0xfc]);
    }
    let mut firmware = vec![0; 49_152];
    firmware[..2].copy_from_slice(&[1, 0]);
    firmware[0x100..0x100 + code.len()].copy_from_slice(&code);
    Machine::with_conditions(
        Images {
            firmware: &firmware,
            eeprom: &[255; 65_536],
            eeprom_status: 0,
            sensor_nonvolatile: None,
        },
        Conditions {
            clocks: Frequencies {
                main_hz: 1_000_000,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn accounting_is_instance_local_and_restoration_starts_a_new_measurement() {
    let mut m = machine(false);
    m.run_until(Time::from_micros(4_000), &[], &mut ()).unwrap();
    assert_eq!(m.ram()[0], 0x2a);
    assert_eq!(m.retired(), 7);
    let before = diagnostic::work(&m);
    assert!(before.cpu_phase_dispatches > 0);
    assert!(before.sensor_sample_phases > 0);
    let exits = before.interval_exits;
    assert_eq!(
        before.interval_entries,
        exits.request
            + exits.committed_owner
            + exits.horizon
            + exits.exception
            + exits.sleep
            + exits.reset
            + exits.error
    );
    let snapshot = m.snapshot();
    let bytes = snapshot.encode().unwrap();
    assert_eq!(
        diagnostic::work(&m),
        before,
        "capture operates on owned copies"
    );
    let decoded = Snapshot::decode(&bytes).unwrap();
    for checkpoint in [&snapshot, &decoded] {
        let mut restored = Machine::from_snapshot(checkpoint);
        assert_eq!(diagnostic::work(&restored), Default::default());
        assert_eq!(restored.ram()[0], 0x2a);
        let (mut original_events, mut restored_events) = (Vec::new(), Vec::new());
        let mut original = m.clone();
        original
            .run_until(Time::from_micros(5_000), &[], &mut original_events)
            .unwrap();
        restored
            .run_until(Time::from_micros(5_000), &[], &mut restored_events)
            .unwrap();
        let interval = diagnostic::work(&original).since(before);
        assert_eq!(interval.cpu_phase_dispatches, 0, "the CPU remains asleep");
        assert_eq!(
            interval.sensor_sample_phases, 12,
            "one millisecond at 12 kHz"
        );
        assert_eq!(original_events, restored_events);
        assert_eq!(
            original.snapshot().encode().unwrap(),
            restored.snapshot().encode().unwrap()
        );
        assert!(diagnostic::work(&restored).sensor_sample_phases > 0);
        assert_eq!(
            diagnostic::work(&m),
            before,
            "running a clone cannot share totals"
        );
    }
    m.restore(&snapshot).unwrap();
    assert_eq!(diagnostic::work(&m), Default::default());
}

#[test]
fn caller_partition_cost_is_excluded_from_causal_state_and_capture_bytes() {
    let (mut whole, mut partitioned) = (machine(true), machine(true));
    let (mut a, mut b) = (Vec::new(), Vec::new());
    whole
        .run_until(Time::from_micros(4_000), &[], &mut a)
        .unwrap();
    let mut at = 0;
    while at < 4_000 {
        at = (at + 113).min(4_000);
        partitioned
            .run_until(Time::from_micros(at), &[], &mut b)
            .unwrap();
    }
    assert_eq!(a, b);
    assert_eq!(
        whole.statistics().bus_reads,
        partitioned.statistics().bus_reads
    );
    assert_eq!(
        whole.statistics().bus_writes,
        partitioned.statistics().bus_writes
    );
    assert_eq!(whole.retired(), partitioned.retired());
    assert_ne!(
        diagnostic::work(&whole).interval_entries,
        diagnostic::work(&partitioned).interval_entries
    );
    assert_eq!(whole.snapshot(), partitioned.snapshot());
    assert_eq!(
        whole.snapshot().encode().unwrap(),
        partitioned.snapshot().encode().unwrap()
    );
}

#[test]
fn hardware_reset_keeps_cumulative_host_work() {
    let mut m = machine(false);
    m.run_until(Time::from_micros(4_000), &[], &mut ()).unwrap();
    let before = diagnostic::work(&m);
    m.run_until(
        Time::from_micros(8_000),
        &[
            TimedInput {
                at: Time::from_micros(4_000),
                input: Input::ResetPin(false),
            },
            TimedInput {
                at: Time::from_micros(5_000),
                input: Input::ResetPin(true),
            },
        ],
        &mut (),
    )
    .unwrap();
    assert_eq!(m.ram()[0], 0x2a);
    assert!(m.statistics().resets > 0);
    let after = diagnostic::work(&m);
    assert!(after.cpu_phase_dispatches > before.cpu_phase_dispatches);
    assert!(after.board_resolutions > before.board_resolutions);
    assert!(after.owner_syncs > before.owner_syncs);
    assert!(after.sensor_sample_phases > before.sensor_sample_phases);
}
