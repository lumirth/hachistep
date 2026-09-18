//! REJ09B0152-0300 §19: the three-bit RES counter counts eight phi edges.
use hs_core::{mcu::clocks::Frequencies, Conditions, Images, Input, Machine, Time, TimedInput};
fn ns(n: u64) -> Time {
    Time::from_raw((u128::from(n) << 64) / 1_000_000_000)
}
fn pin(n: u64, high: bool) -> TimedInput {
    TimedInput {
        at: ns(n),
        input: Input::ResetPin(high),
    }
}
fn machine() -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&[1, 0]);
    rom[0x100..0x102].copy_from_slice(&[0x40, 0xfe]);
    Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[0xff; 65536],
            eeprom_status: 0,
        },
        Conditions {
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
fn reset_release_counts_edges_and_reassertion_discards_partial_qualification() {
    for reassert in [false, true] {
        let mut m = machine();
        let mut changes = vec![pin(0, false), pin(125, true)];
        if reassert {
            changes.extend([pin(1600, false), pin(3125, true)]);
        }
        let first_read = ns(if reassert { 5500 } else { 2500 });
        let mut split = m.clone();
        let (mut a, mut b) = (vec![], vec![]);
        m.run_until(first_read, &changes, &mut a).unwrap();
        assert_eq!(
            m.statistics().bus_reads,
            0,
            "vector transfer is still pending at the exclusive horizon"
        );
        let mut consumed = 0;
        for n in (100..first_read.as_micros() as u64 * 1000).step_by(100) {
            consumed += split
                .run_until(ns(n), &changes[consumed..], &mut b)
                .unwrap()
                .inputs_consumed;
            split = Machine::from_snapshot(&split.snapshot());
        }
        split
            .run_until(first_read, &changes[consumed..], &mut b)
            .unwrap();
        assert_eq!(m, split);
        assert_eq!(a, b);
        m.run_until(Time::from_raw(first_read.raw() + 1), &[], &mut a)
            .unwrap();
        assert_eq!(m.statistics().bus_reads, 1);
    }
}
