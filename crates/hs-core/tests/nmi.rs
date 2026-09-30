//! REJ09B0152-0300 §§3.4.1, 3.5.1, 3.8.6, 5.2 and 6.3.
//! Electrical fixture pulses below are much longer than the two-clock minimum.
#[path = "support/state.rs"]
mod state;
use hs_core::{diagnostic::mcu::control::Control, Images, Input, Machine, Time, TimedInput};
fn t(us: u64) -> Time {
    Time::from_micros(us)
}
fn program(code: &[u8]) -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
    rom[14..16].copy_from_slice(&0x200u16.to_be_bytes());
    rom[0x100..0x100 + code.len()].copy_from_slice(code);
    // The guest interrupt handler increments RAM, then executes RTE.
    rom[0x200..0x20c].copy_from_slice(&[
        0x6a, 0x08, 0xf7, 0x80, 0x0a, 0x08, 0x6a, 0x88, 0xf7, 0x80, 0x56, 0x70,
    ]);
    Machine::new(Images {
        firmware: &rom,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap()
}
fn input(us: u64, high: bool) -> TimedInput {
    TimedInput {
        at: t(us),
        input: Input::NmiPin(high),
    }
}

#[test]
fn nmi_during_reset_fetch_waits_for_the_first_stack_initializing_instruction() {
    let mut m = program(&[0x7a, 7, 0, 0, 0xff, 0x70, 0x40, 0xfe]);
    m.run_until(t(50), &[input(1, false)], &mut ()).unwrap();
    assert_eq!(m.interrupt_entries(), 1);
    assert_eq!(m.registers().sp(), 0xff70);
    assert_eq!(&m.ram()[0x7ec..0x7f0], &[0x80, 0x80, 1, 6]);
    assert_eq!(m.ram()[0], 1);
}

#[test]
fn dedicated_edge_latch_is_independent_of_ien_and_irr() {
    let mut c = Control::default();
    c.nmi_input(false, true);
    assert!(c.nmi_pending());
    c.write(0xfff3, 0).unwrap();
    c.write(0xfff4, 0).unwrap();
    c.write(0xfff6, 0).unwrap();
    c.write(0xfff7, 0).unwrap();
    assert!(c.nmi_pending());
    c.acknowledge_nmi();
    c.nmi_input(false, true);
    assert!(
        !c.nmi_pending(),
        "a held level must not reassert an edge request"
    );
    c.write(0xfff2, 0x80).unwrap();
    assert!(
        !c.nmi_pending(),
        "changing edge selection is not an input edge"
    );
    c.nmi_input(true, true);
    assert!(c.nmi_pending());
    c.reset();
    assert!(!c.nmi_pending());
    assert!(c.nmi_level());
    c.nmi_input(false, false);
    c.reset();
    assert!(
        !c.nmi_level(),
        "reset cannot drive the external NMI pad high"
    );
}

#[test]
fn masked_sleep_wakes_and_a_held_nmi_is_not_repeated() {
    let code = [0x79, 7, 0xff, 0x70, 0x07, 0x80, 0x01, 0x80, 0x40, 0xfc];
    let mut m = program(&code);
    m.run_until(t(100), &[], &mut ()).unwrap();
    assert_eq!(m.registers().ccr & 0x80, 0x80);
    let changes = [input(100, false), input(300, true), input(500, false)];
    let mut events = Vec::new();
    m.run_until(t(200), &changes, &mut events).unwrap();
    assert_eq!(m.ram()[0], 1);
    assert_eq!(m.interrupt_entries(), 1);
    m.run_until(t(800), &changes[1..], &mut events).unwrap();
    assert_eq!(m.ram()[0], 2);
    assert_eq!(m.interrupt_entries(), 2);
    assert_eq!(m.registers().ccr & 0x80, 0x80);
    let snap = m.snapshot();
    let duplicate = [input(800, true), input(800, false)];
    assert!(m.run_until(t(900), &duplicate, &mut ()).is_err());
    assert_eq!(m.snapshot(), snap);
}

#[test]
fn nmi_wake_respects_standby_stabilization_and_partitioning() {
    // Configure standby (SSBY=1,TMA3=0,LSON=0), retain I=1.
    let code = [
        0x79, 7, 0xff, 0x70, 0xf8, 0x80, 0x38, 0xf0, 0x01, 0x80, 0x40, 0xfc,
    ];
    let mut long = program(&code);
    let mut short = long.clone();
    let changes = [input(100, false), input(6000, true)];
    let end = t(7000);
    let mut a = Vec::new();
    let mut b = Vec::new();
    long.run_until(end, &changes, &mut a).unwrap();
    let mut consumed = 0;
    for us in 1..=7000 {
        consumed += short
            .run_until(t(us), &changes[consumed..], &mut b)
            .unwrap()
            .inputs_consumed;
        if us == 120 {
            short = state::restore_file(&short.snapshot());
        }
    }
    state::assert_same_state(&long, &short);
    assert_eq!(a, b);
    assert_eq!(long.ram()[0], 1);
}

#[test]
fn non_user_reset_modes_report_an_error_and_preserve_the_stopped_session() {
    for strap in [
        Input::NmiPin(false),
        Input::DigitalPin {
            pin: hs_core::DigitalPin::Adtrg,
            level: Some(true),
        },
    ] {
        let mut m = program(&[0x40, 0xfe]);
        let firmware = m.firmware();
        let events = [
            TimedInput {
                at: t(10),
                input: Input::ResetPin(false),
            },
            TimedInput {
                at: t(10),
                input: strap,
            },
            TimedInput {
                at: t(100),
                input: Input::ResetPin(true),
            },
        ];
        assert_eq!(
            m.run_until(t(200), &events, &mut ()).unwrap_err(),
            hs_core::Error::UnsupportedResetMode,
        );
        assert_eq!(m.firmware(), firmware);
        assert_eq!(m.interrupt_entries(), 0);
        let stopped = m.snapshot();
        assert!(m.run_until(t(300), &[], &mut ()).is_err());
        assert_eq!(m.snapshot(), stopped);
        let mut loaded = state::restore_file(&stopped);
        assert!(loaded.run_until(t(300), &[], &mut ()).is_err());
    }
}

#[test]
fn low_nmi_during_a_short_power_dip_is_neither_an_edge_nor_a_reset_strap() {
    let mut m = program(&[0x40, 0xfe]);
    m.power_off(&mut ()).unwrap();
    m.run_until(t(1), &[input(0, false)], &mut ()).unwrap();
    m.power_on(&mut ()).unwrap();
    m.run_until(t(400), &[], &mut ()).unwrap();
    assert!(m.powered());
    assert_eq!(m.statistics().resets, 0);
    assert_eq!(m.interrupt_entries(), 0);
    assert!(m.retired() > 0);
}
