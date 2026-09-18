//! Reset timing at the embedding boundary, including an interrupted CPU and
//! external RES overlap. Register expectations are exercised by hachiware.
#[path = "support/state.rs"]
mod state;
use hs_core::{
    mcu::clocks::Frequencies, Conditions, Event, Images, Input, Machine, Time, TimedInput,
};

fn machine() -> Machine {
    let mut rom = [0; 49152];
    rom[..2].copy_from_slice(&0x100_u16.to_be_bytes());
    // TCWE=1; preload FF. The first private ROSC/2048 edge asserts reset.
    let code = [0xf8, 0x5e, 0x38, 0xb1, 0xf8, 0xff, 0x38, 0xb3, 0x40, 0xfe];
    rom[0x100..0x100 + code.len()].copy_from_slice(&code);
    Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[0xff; 65536],
            eeprom_status: 0,
        },
        Conditions {
            clocks: Frequencies {
                main_hz: 1_000_000,
                on_chip_hz: 1_000_000,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn watchdog_holds_reset_for_512_rosc_edges_and_restores_mid_hold() {
    let mut m = machine();
    let mut events = Vec::new();
    m.run_until(Time::from_micros(2049), &[], &mut events)
        .unwrap();
    assert!(events.contains(&Event::Reset {
        at: Time::from_micros(2048),
        watchdog: true
    }));
    assert_eq!(m.peek(0xffb1).unwrap(), 0xaf);
    assert_eq!(m.peek(0xffb3).unwrap(), 0);
    assert_eq!(m.retired(), 0);
    let reads = m.statistics().bus_reads;
    let mut restored = state::restore_file(&m.snapshot());
    let mut a = Vec::new();
    let mut b = Vec::new();
    m.run_until(Time::from_micros(2560), &[], &mut a).unwrap();
    assert_eq!(m.statistics().bus_reads, reads);
    assert_eq!(m.peek(0xffb3).unwrap(), 0);
    m.run_until(Time::from_micros(2600), &[], &mut a).unwrap();
    for us in (2050..2600).step_by(17).chain([2600]) {
        restored
            .run_until(Time::from_micros(us), &[], &mut b)
            .unwrap();
    }
    assert_eq!(a, b);
    state::assert_same_state(&m, &restored);
    assert!(m.statistics().bus_reads > reads);
    assert!(m.retired() > 0);
}

#[test]
fn external_reset_clears_cause_without_rephasing_the_internal_hold() {
    for release in [2400, 2700] {
        let mut m = machine();
        m.run_until(Time::from_micros(2049), &[], &mut ()).unwrap();
        let reads = m.statistics().bus_reads;
        let inputs = [
            TimedInput {
                at: Time::from_raw(Time::from_micros(2100).raw() + (1_u128 << 64) / 4_000_000),
                input: Input::ResetPin(false),
            },
            TimedInput {
                at: Time::from_micros(release),
                input: Input::ResetPin(true),
            },
        ];
        let end = (release + 8).max(2560);
        let consumed = m
            .run_until(Time::from_micros(end), &inputs, &mut ())
            .unwrap()
            .inputs_consumed;
        assert_eq!(m.peek(0xffb1).unwrap(), 0xae);
        assert_eq!(m.statistics().bus_reads, reads);
        // Reset vector fetch is the first bus effect, two phi edges after
        // the final reset contribution releases. RES takes eight phi edges;
        // WDT does not acquire another eight-edge hold. It precedes all guest code.
        m.run_until(Time::from_micros(end + 2), &inputs[consumed..], &mut ())
            .unwrap();
        assert_eq!(m.statistics().bus_reads, reads);
        m.run_until(
            Time::from_raw(Time::from_micros(end + 2).raw() + 1),
            &[],
            &mut (),
        )
        .unwrap();
        assert_eq!(m.statistics().bus_reads, reads + 1);
    }
}
