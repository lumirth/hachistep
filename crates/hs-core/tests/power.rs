//! Package rules and the explicit nominal RC/startup/retention realization.
#[path = "support/state.rs"]
mod state;
use hs_core::{mcu::clocks::Frequencies, Conditions, Images, Input, Machine, Time, TimedInput};

fn t(us: u64) -> Time {
    Time::from_micros(us)
}
fn after(at: Time) -> Time {
    Time::from_raw(at.raw() + 1)
}
fn rail(at: Time, mv: u16) -> TimedInput {
    TimedInput {
        at,
        input: Input::SupplyMillivolts(mv),
    }
}
fn machine(mv: u16) -> Machine {
    // Leave a RAM marker and a stopped RTC minute value, then execute in ROM.
    let code = [
        0xf8, 0xa5, 0x6a, 0x88, 0xf7, 0x80, 0xf8, 0x23, 0x6a, 0x88, 0xf0, 0x69, 0x40, 0xfe,
    ];
    from_code(mv, &code)
}
fn from_code(mv: u16, code: &[u8]) -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&[1, 0]);
    rom[0x100..0x100 + code.len()].copy_from_slice(code);
    Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[0xff; 65536],
            eeprom_status: 0,
        },
        Conditions {
            supply_millivolts: mv,
            clocks: Frequencies {
                main_hz: 4_000_000,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn cold_watch_acquisition_does_not_delay_main_execution_or_backfill_rtc_ticks() {
    let mut m = from_code(
        0,
        &[0xf8, 0xc8, 0x6a, 0x88, 0xf0, 0x6c, 0x01, 0x80, 0x40, 0xfe],
    );
    m.run_until(t(100_000), &[rail(t(0), 3000)], &mut ())
        .unwrap();
    assert!(
        m.sleeping(),
        "main-clock code already configured the RTC and slept"
    );
    assert_eq!(m.peek(0xf068).unwrap(), 0);
    m.run_until(t(2_500_000), &[], &mut ()).unwrap();
    assert_eq!(
        m.peek(0xf068).unwrap(),
        0,
        "watch counted only half a second"
    );
    m.run_until(t(3_000_010), &[], &mut ()).unwrap();
    assert_eq!(m.peek(0xf068).unwrap(), 1);
}
#[test]
fn cold_res_qualification_precedes_the_real_flash_vector_transfer() {
    let mut m = machine(0);
    let mut split = m.clone();
    let changes = [rail(t(0), 3000)];
    // 100 kOhm * 100 nF * ln(5), then eight 4-MHz phi edges,
    // then the documented two-state reset-vector transfer.
    let first = Time::from_raw((16_096_750u128 << 64) / 1_000_000_000);
    let (mut a, mut b) = (vec![], vec![]);
    m.run_until(first, &changes, &mut a).unwrap();
    assert_eq!(m.statistics().bus_reads, 0);
    let mut consumed = 0;
    for us in (1..16096).step_by(173) {
        consumed += split
            .run_until(t(us), &changes[consumed..], &mut b)
            .unwrap()
            .inputs_consumed;
        split = state::restore_file(&split.snapshot());
    }
    split
        .run_until(first, &changes[consumed..], &mut b)
        .unwrap();
    state::assert_same_state(&m, &split);
    assert_eq!(a, b);
    m.run_until(after(first), &[], &mut a).unwrap();
    assert_eq!(m.statistics().bus_reads, 1);
    m.run_until(t(16200), &[], &mut a).unwrap();
    assert_eq!(
        m.ram()[0],
        0xa5,
        "the actual flash vector reaches the original guest"
    );
}
#[test]
fn a_short_collapse_preserves_execution_and_ram_but_five_ms_requires_res() {
    for (absence, resets) in [(1000, 0), (5000, 1)] {
        let mut m = machine(3000);
        m.run_until(t(100), &[], &mut ()).unwrap();
        let retired = m.retired();
        let changes = [rail(t(100), 0), rail(t(100 + absence), 3000)];
        let mut split = m.clone();
        let (mut a, mut b) = (vec![], vec![]);
        m.run_until(t(100 + absence), &changes, &mut a).unwrap();
        assert_eq!(m.retired(), retired);
        assert_eq!(m.ram()[0], 0xa5);
        assert_eq!(m.peek(0xf069).unwrap(), 0x23);
        m.run_until(after(t(100 + absence)), &changes[1..], &mut a)
            .unwrap();
        assert_eq!(m.statistics().resets, resets);
        assert_eq!(m.ram()[0], 0xa5);
        let end = t(100 + absence + 10000);
        m.run_until(end, &[], &mut a).unwrap();
        let mut consumed = 0;
        for us in (121..100 + absence + 10000).step_by(337) {
            consumed += split
                .run_until(t(us), &changes[consumed..], &mut b)
                .unwrap()
                .inputs_consumed;
            split = Machine::from_snapshot(&split.snapshot());
        }
        split.run_until(end, &changes[consumed..], &mut b).unwrap();
        state::assert_same_state(&m, &split);
        assert_eq!(a, b);
        assert_eq!(m.instruction_pc(), 0x10c);
    }
}
#[test]
fn retention_loss_is_a_physical_elapsed_time_boundary_even_with_no_cpu_clock() {
    for (mv, duration) in [(0, 10000), (1000, 30000), (1400, 150000)] {
        let mut m = machine(3000);
        m.run_until(t(100), &[], &mut ()).unwrap();
        m.run_until(t(100 + duration - 1), &[rail(t(100), mv)], &mut ())
            .unwrap();
        assert_eq!(m.ram()[0], 0xa5);
        assert_eq!(m.peek(0xf069).unwrap(), 0x23);
        let mut restored = Machine::from_snapshot(&m.snapshot());
        m.run_until(t(101 + duration), &[], &mut ()).unwrap();
        restored.run_until(t(101 + duration), &[], &mut ()).unwrap();
        state::assert_same_state(&m, &restored);
        assert_eq!(m.ram()[0], 0);
        assert_eq!(m.peek(0xf069).unwrap(), 0);
        assert!(!m.powered() || mv != 0);
    }
}
