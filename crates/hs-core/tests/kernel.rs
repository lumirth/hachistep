#[path = "support/state.rs"]
mod state;
use hs_core::{Buttons, DigitalPin, Event, Images, Input, Machine, Time, TimedInput};
fn machine(code: &[u8]) -> Machine {
    let mut flash = vec![0u8; 49152];
    flash[..2].copy_from_slice(&0x0100u16.to_be_bytes());
    flash[0x100..0x100 + code.len()].copy_from_slice(code);
    Machine::new(Images {
        firmware: &flash,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
    })
    .unwrap()
}
const LOOP: &[u8] = &[
    0x79, 0x07, 0xff, 0x80, 0xf8, 0x2a, 0x6a, 0x88, 0xf7, 0x80, 0x0a, 0x08, 0x40, 0xf8,
];
#[test]
fn watch_counter_keeps_the_last_tick_of_oscillator_stabilization() {
    // Timer B1 uses the independent watch source. NMI restarts the 1-MHz
    // main oscillator at 258 us; STS=000 ends its 8192-cycle wait at 8450 us.
    // The watch/256 edge at 8448 us must survive that gate transition.
    let mut code = vec![0x79, 7, 0xff, 0x70];
    for (a, v) in [
        (0xfffa_u16, 7), // Keep flash available while enabling Timer B1.
        (0xf0d0, 0x3f),
        (0xf0d1, 0),
        (0xf0d0, 0x7f),
        (0xfff0, 0x84),
    ] {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    code.extend([0x01, 0x80, 0x40, 0xfe]);
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
    rom[14..16].copy_from_slice(&0x200u16.to_be_bytes());
    rom[0x100..0x100 + code.len()].copy_from_slice(&code);
    rom[0x200..0x202].copy_from_slice(&[0x40, 0xfe]);
    let mut a = Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[0xff; 65536],
            eeprom_status: 0,
        },
        hs_core::Conditions {
            clocks: hs_core::mcu::clocks::Frequencies {
                main_hz: 1_000_000,
                watch_hz: 1_000_000,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let mut b = a.clone();
    let wake = TimedInput {
        at: Time::from_micros(258),
        input: Input::NmiPin(false),
    };
    let harmless = TimedInput {
        at: Time::from_micros(8449),
        input: Input::SupplyMillivolts(3000),
    };
    a.run_until(Time::from_micros(8705), &[wake], &mut ())
        .unwrap();
    b.run_until(Time::from_micros(8705), &[wake, harmless], &mut ())
        .unwrap();
    assert_eq!(a.peek(0xf0d1).unwrap(), 34);
    assert_eq!(b.peek(0xf0d1).unwrap(), 34);
    assert_eq!(a.registers(), b.registers());
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(32))]
    #[test]
    fn run_partition_and_snapshot_replay_preserve_all_state_and_events(
        mut horizons in proptest::collection::vec(1u64..5000, 0..16),
        checkpoint in 0u64..=5000,
    ) {
        let mut long = machine(LOOP);
        let mut short = long.clone();
        let mut a = Vec::new();
        let mut b = Vec::new();
        long.run_until(Time::from_micros(5000), &[], &mut a).unwrap();
        horizons.extend([checkpoint, 5000]);
        horizons.sort_unstable();
        horizons.dedup();
        for us in horizons {
            short.run_until(Time::from_micros(us), &[], &mut b).unwrap();
            if us == checkpoint {
                short = state::restore_file(&short.snapshot());
            }
        }
        proptest::prop_assert_eq!(a, b);
        state::assert_same_state(&long, &short);
    }
}
#[test]
fn horizon_and_input_timestamps_are_exclusive() {
    let mut m = machine(LOOP);
    let change = TimedInput {
        at: Time::from_micros(12),
        input: Input::Buttons(Buttons {
            center: true,
            left: false,
            right: false,
        }),
    };
    let result = m.run_until(change.at, &[change], &mut ()).unwrap();
    assert_eq!(result.inputs_consumed, 0);
    assert_eq!(m.peek(0xffde).unwrap() & 1, 0);
    let result = m
        .run_until(Time::from_micros(13), &[change], &mut ())
        .unwrap();
    assert_eq!(result.inputs_consumed, 1);
    assert_eq!(m.peek(0xffde).unwrap() & 1, 1);
}
#[test]
fn external_serial_edges_survive_partition_and_restore_inside_a_byte() {
    let mut code = Vec::new();
    for (address, value) in [
        (0xfffbu16, 0x14),
        (0xf0e0, 0x0d),
        (0xf0e1, 0x40),
        (0xf0e2, 0x80),
        (0xf0e3, 0x40),
    ] {
        code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
    }
    code.extend([0x40, 0xfe]);
    let pin = |us, pin, high| TimedInput {
        at: Time::from_micros(us),
        input: Input::DigitalPin {
            pin,
            level: Some(high),
        },
    };
    let mut inputs = vec![
        pin(0, DigitalPin::P90, true),
        pin(0, DigitalPin::P91, true),
        pin(500, DigitalPin::P90, false),
    ];
    for bit in 0..8 {
        inputs.extend([
            pin(515 + 20 * bit, DigitalPin::P92, 0x96 & (0x80 >> bit) != 0),
            pin(520 + 20 * bit, DigitalPin::P91, false),
            pin(530 + 20 * bit, DigitalPin::P91, true),
        ]);
    }
    inputs.push(pin(680, DigitalPin::P90, true));
    let mut whole = machine(&code);
    let mut split = whole.clone();
    let mut a = Vec::new();
    let mut b = Vec::new();
    whole
        .run_until(Time::from_micros(1000), &inputs, &mut a)
        .unwrap();
    let mut consumed = 0;
    for us in (1..1000).step_by(7).chain([1000]) {
        consumed += split
            .run_until(Time::from_micros(us), &inputs[consumed..], &mut b)
            .unwrap()
            .inputs_consumed;
        if (520..670).contains(&us) {
            split = Machine::from_snapshot(&split.snapshot());
        }
    }
    assert_eq!(whole.peek(0xf0e9).unwrap(), 0x96);
    state::assert_same_state(&whole, &split);
    assert_eq!(a, b);
}
#[test]
fn sci_pin_edges_and_buffered_characters_survive_partition_and_restore() {
    let mut code = vec![];
    for (a, v) in [
        (0xffd6_u16, 1),
        (0xffe6, 5),
        (0xfffa, 0x43),
        (0xff91, 0xd0),
        (0xff99, 1),
        (0xffa7, 0x80),
        (0xff9a, 0x20),
        (0xff9b, 0xa5),
        (0xffd6, 0),
    ] {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    for value in [0x3c, 0xc3] {
        code.extend([0x6a, 0x08, 0xff, 0x9c, 0xe8, 0x80, 0x47, 0xf8]);
        code.extend([0xf8, value, 0x6a, 0x88, 0xff, 0x9b]);
    }
    code.extend([0x40, 0xfe]);
    let mut whole = machine(&code);
    let mut split = whole.clone();
    let (mut a, mut b) = (vec![], vec![]);
    whole
        .run_until(Time::from_micros(1000), &[], &mut a)
        .unwrap();
    for us in (1..1000).step_by(3).chain([1000]) {
        split.run_until(Time::from_micros(us), &[], &mut b).unwrap();
        split = Machine::from_snapshot(&split.snapshot());
    }
    assert_eq!(a, b);
    state::assert_same_state(&whole, &split);
    assert_eq!(
        a.iter()
            .filter(|e| matches!(e, Event::Infrared { .. }))
            .count(),
        30
    );
    assert_eq!(whole.peek(0xff9c).unwrap(), 0x84);

    let mut code = vec![];
    for (a, v) in [
        (0xfffa_u16, 0x43),
        (0xff91, 0xd0),
        (0xff98, 0x80),
        (0xff99, 0),
        (0xff9a, 0x32),
        (0xff9b, 0xa5),
    ] {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    code.extend([0x40, 0xfe]);
    let pin = |us, pin, level| TimedInput {
        at: Time::from_micros(us),
        input: Input::DigitalPin {
            pin,
            level: Some(level),
        },
    };
    let mut inputs = vec![
        pin(0, DigitalPin::P30, true),
        pin(0, DigitalPin::P31, false),
    ];
    for i in 0..8 {
        inputs.extend([
            pin(500 + 100 * i, DigitalPin::P31, 0x3c & (1 << i) != 0),
            pin(500 + 100 * i, DigitalPin::P30, false),
            pin(550 + 100 * i, DigitalPin::P30, true),
        ]);
    }
    let mut whole = machine(&code);
    let mut split = whole.clone();
    let (mut a, mut b) = (vec![], vec![]);
    whole
        .run_until(Time::from_micros(1500), &inputs, &mut a)
        .unwrap();
    let mut consumed = 0;
    for us in (1..1500).step_by(17).chain([1500]) {
        consumed += split
            .run_until(Time::from_micros(us), &inputs[consumed..], &mut b)
            .unwrap()
            .inputs_consumed;
        split = Machine::from_snapshot(&split.snapshot());
    }
    assert_eq!(a, b);
    state::assert_same_state(&whole, &split);
    assert_eq!(whole.peek(0xff9d).unwrap(), 0x3c);
    assert_eq!(whole.peek(0xff9c).unwrap(), 0xc4);
    assert_eq!(
        a.iter()
            .filter(|e| matches!(e, Event::Infrared { .. }))
            .count(),
        8
    );
}
#[test]
fn iic_package_inputs_preserve_frames_through_partition_and_restore() {
    let mut code = vec![0x79, 7, 0xff, 0x70];
    for (a, v) in [
        (0xfffb_u16, 0x24),
        (0xf087, 3),
        (0xf07d, 0x54),
        (0xf078, 0x80),
    ] {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    for slot in 0..3 {
        code.extend([0x6a, 8, 0xf0, 0x7c, 0xe8, 0x20, 0x47, 0xf8]);
        code.extend([0x6a, 8, 0xf0, 0x7f, 0x6a, 0x88, 0xf8, slot]);
    }
    code.extend([0x40, 0xfe]);
    let mut inputs = vec![];
    let mut drive = |us, pin, high| {
        inputs.push(TimedInput {
            at: Time::from_micros(us),
            input: Input::DigitalPin {
                pin,
                level: Some(high),
            },
        })
    };
    drive(0, DigitalPin::P90, true);
    drive(0, DigitalPin::P91, true);
    drive(100, DigitalPin::P91, false);
    let mut us = 120;
    for byte in [0x54, 0x3c, 0xa5] {
        for bit in (0..8).rev() {
            drive(us, DigitalPin::P90, false);
            drive(us, DigitalPin::P91, byte & (1 << bit) != 0);
            drive(us + 20, DigitalPin::P90, true);
            us += 40;
        }
        drive(us, DigitalPin::P90, false);
        drive(us, DigitalPin::P91, true);
        drive(us + 20, DigitalPin::P90, true);
        us += 40;
    }
    let mut whole = machine(&code);
    let mut split = whole.clone();
    let (mut a, mut b) = (vec![], vec![]);
    whole
        .run_until(Time::from_micros(1500), &inputs, &mut a)
        .unwrap();
    let mut consumed = 0;
    for us in (1..1500).step_by(7).chain([1500]) {
        consumed += split
            .run_until(Time::from_micros(us), &inputs[consumed..], &mut b)
            .unwrap()
            .inputs_consumed;
        split = Machine::from_snapshot(&split.snapshot());
    }
    assert_eq!(&whole.ram()[0x80..0x83], &[0x54, 0x3c, 0xa5]);
    assert_eq!(a, b);
    state::assert_same_state(&whole, &split);
}

#[test]
fn external_avcc_fixture_sets_the_adc_midpoint_transitions() {
    // 2048 mV / 1024 gives 2 mV per code, with transitions at odd millivolts.
    let code = [
        0xf8, 0x13, 0x6a, 0x88, 0xff, 0xfa, // ADC module clock
        0xf8, 4, 0x6a, 0x88, 0xff, 0xbe, // PB0, phi/4
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // module settling
        0xf8, 0x80, 0x6a, 0x88, 0xff, 0xbf, 0x40, 0xfe,
    ];
    let base = machine(&code);
    for (mv, expected) in [(0, 0), (1, 1), (2, 1), (3, 2), (2047, 1023)] {
        let mut m = Machine::with_conditions(
            Images {
                firmware: &*base.firmware(),
                eeprom: &base.eeprom(),
                eeprom_status: 0,
            },
            hs_core::Conditions {
                avcc_override_millivolts: Some(2048),
                ..Default::default()
            },
        )
        .unwrap();
        m.run_until(
            Time::from_micros(500),
            &[TimedInput {
                at: Time::ZERO,
                input: Input::AnalogPin {
                    pin: hs_core::AnalogPin::Pb0,
                    millivolts: Some(mv),
                },
            }],
            &mut (),
        )
        .unwrap();
        let result = u16::from_be_bytes([m.peek(0xffbc).unwrap(), m.peek(0xffbd).unwrap()]);
        assert_eq!(result, expected << 6, "input {mv} mV");
    }
}

#[test]
fn adc_trigger_and_held_sample_survive_partition_and_restore() {
    let mut code = vec![];
    for (a, v) in [
        (0xfffa_u16, 0x13),
        (0xffca, 8),
        (0xfff2, 0x20),
        (0xffbe, 0x74),
    ] {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    code.extend([0x40, 0xfe]);
    let analog = |us, millivolts| TimedInput {
        at: Time::from_micros(us),
        input: Input::AnalogPin {
            pin: hs_core::AnalogPin::Pb0,
            millivolts: Some(millivolts),
        },
    };
    let trigger = |us, high| TimedInput {
        at: Time::from_micros(us),
        input: Input::DigitalPin {
            pin: DigitalPin::Adtrg,
            level: Some(high),
        },
    };
    // Both input changes occur after acquisition, before conversion completes.
    let inputs = [
        analog(0, 0),
        trigger(500, true),
        analog(650, 5000),
        trigger(1700, false),
        trigger(2000, true),
        analog(2300, 0),
    ];
    let mut whole = machine(&code);
    let mut split = whole.clone();
    let (mut a, mut b) = (vec![], vec![]);
    let consumed = whole
        .run_until(Time::from_micros(1800), &inputs, &mut a)
        .unwrap()
        .inputs_consumed;
    assert_eq!(whole.peek(0xffbc).unwrap(), 0);
    whole
        .run_until(Time::from_micros(4000), &inputs[consumed..], &mut a)
        .unwrap();
    let mut consumed = 0;
    for us in (1..4000).step_by(11).chain([4000]) {
        consumed += split
            .run_until(Time::from_micros(us), &inputs[consumed..], &mut b)
            .unwrap()
            .inputs_consumed;
        split = Machine::from_snapshot(&split.snapshot());
    }
    assert_eq!(whole.peek(0xffbc).unwrap(), 0xff);
    assert_eq!(whole.peek(0xffbd).unwrap(), 0xc0);
    state::assert_same_state(&whole, &split);
    assert_eq!(a, b);
}
#[test]
fn rtc_clock_output_is_a_physical_pin_even_with_the_counter_stopped() {
    let mut code = vec![];
    for (a, v) in [(0xf06f_u16, 0x18), (0xffc0, 2)] {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    code.extend([0x40, 0xfe]);
    let base = machine(&code);
    let mut whole = Machine::with_conditions(
        Images {
            firmware: &*base.firmware(),
            eeprom: &base.eeprom(),
            eeprom_status: 0,
        },
        hs_core::Conditions {
            clocks: hs_core::mcu::clocks::Frequencies {
                main_hz: 1_000_000,
                watch_hz: 1000,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let mut split = whole.clone();
    for (us, level) in [(499, 1), (501, 0), (1001, 1), (1501, 0)] {
        whole
            .run_until(Time::from_micros(us), &[], &mut ())
            .unwrap();
        assert_eq!(whole.peek(0xffd4).unwrap() & 1, level);
        assert_eq!(whole.peek(0xf06c).unwrap() & 0x80, 0);
    }
    whole
        .run_until(Time::from_micros(2001), &[], &mut ())
        .unwrap();
    for us in (1..2001).step_by(37).chain([2001]) {
        split
            .run_until(Time::from_micros(us), &[], &mut ())
            .unwrap();
        split = Machine::from_snapshot(&split.snapshot());
    }
    state::assert_same_state(&whole, &split);
}
#[test]
fn invalid_timeline_is_rejected_before_mutation() {
    let mut m = machine(LOOP);
    let before = m.snapshot();
    let duplicate = [TimedInput {
        at: Time::ZERO,
        input: Input::Buttons(Buttons::RELEASED),
    }; 2];
    assert!(m
        .run_until(Time::from_micros(100), &duplicate, &mut ())
        .is_err());
    assert_eq!(before, m.snapshot());
    let backward = [
        TimedInput {
            at: Time::from_micros(2),
            input: Input::SupplyMillivolts(3000),
        },
        TimedInput {
            at: Time::from_micros(1),
            input: Input::SupplyMillivolts(2900),
        },
    ];
    assert!(m
        .run_until(Time::from_micros(100), &backward, &mut ())
        .is_err());
    assert_eq!(before, m.snapshot());
}
#[test]
fn instruction_fetches_from_ram_follow_the_same_executor() {
    let mut code = vec![0x79, 0x07, 0xff, 0x80];
    // Fill RAM with MOV.B #42,R0L; BRA . using ordinary guest stores.
    for (a, v) in [
        (0xf780u16, 0xf8),
        (0xf781, 42),
        (0xf782, 0x40),
        (0xf783, 0xfe),
    ] {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    code.extend([0x5a, 0, 0xf7, 0x80]);
    let mut m = machine(&code);
    m.run_until(Time::from_micros(100), &[], &mut ()).unwrap();
    assert_eq!(m.registers().er[0] & 255, 42);
    assert_eq!(m.instruction_pc(), 0xf782);
}
#[test]
fn invalid_instruction_latches_a_fault_without_erasing_prior_store() {
    let code = [0xf8, 0x5a, 0x6a, 0x88, 0xf7, 0x80, 0x57, 0xff];
    let mut m = machine(&code);
    let healthy = m.snapshot();
    let error = m
        .run_until(Time::from_micros(100), &[], &mut ())
        .unwrap_err();
    assert_eq!(m.ram()[0], 0x5a);
    assert!(m.fault().is_some());
    let stopped = m.snapshot();
    let mut events = vec![];
    assert_eq!(m.power_on(&mut events), Err(error.clone())); // Even a redundant call.
    assert_eq!(m.power_off(&mut events), Err(error.clone()));
    let reset = [TimedInput {
        at: m.now(),
        input: Input::ResetPin(false),
    }];
    assert_eq!(
        m.run_until(Time::from_micros(200), &reset, &mut events),
        Err(error)
    );
    assert_eq!(m.snapshot(), stopped);
    assert!(events.is_empty());
    m.restore(&healthy).unwrap();
    assert_eq!(m.fault(), None);
    assert_eq!(m.snapshot(), healthy);
    m.run_until(Time::ZERO, &[], &mut ()).unwrap();
}

#[test]
fn immediate_power_failure_latches_before_any_further_transition() {
    let mut m = machine(LOOP);
    m.power_off(&mut ()).unwrap();
    m.run_until(Time::MAX, &[], &mut ()).unwrap();
    let restored = state::restore_file(&m.snapshot());
    assert_eq!(restored.now(), Time::MAX);
    assert!(!restored.powered());
    state::assert_same_state(&m, &restored);
    let mut events = vec![];
    let error = m.power_on(&mut events).unwrap_err();
    assert_eq!(m.fault(), Some(&error));
    let stopped = m.snapshot();
    events.clear();
    assert_eq!(m.power_off(&mut events), Err(error.clone()));
    assert_eq!(m.power_on(&mut events), Err(error.clone()));
    assert_eq!(m.run_until(Time::MAX, &[], &mut events), Err(error));
    assert_eq!(m.snapshot(), stopped);
    assert!(events.is_empty());
}
#[test]
fn reset_pin_aborts_cpu_work_but_keeps_existing_ram() {
    let mut m = machine(LOOP);
    m.run_until(Time::from_micros(10), &[], &mut ()).unwrap();
    let old = m.ram()[0];
    let inputs = [
        TimedInput {
            at: Time::from_micros(10),
            input: Input::ResetPin(false),
        },
        TimedInput {
            at: Time::from_micros(100),
            input: Input::ResetPin(true),
        },
    ];
    let mut events = Vec::new();
    m.run_until(Time::from_micros(99), &inputs, &mut events)
        .unwrap();
    assert_eq!(m.ram()[0], old);
    assert_eq!(m.retired(), 0);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Reset {
            watchdog: false,
            ..
        }
    )));
    m.run_until(Time::from_micros(120), &inputs[1..], &mut events)
        .unwrap();
    assert!(m.retired() > 0);
}
#[test]
fn peeking_does_not_change_causal_state() {
    let mut m = machine(LOOP);
    m.run_until(Time::from_micros(100), &[], &mut ()).unwrap();
    let before = m.snapshot();
    for a in [0xf0e4, 0xf0e9, 0xffb1, 0xf068, 0xffde, 0xf780] {
        let _ = m.peek(a);
    }
    assert_eq!(m.snapshot(), before);
}

#[test]
fn guest_serial_page_write_reaches_the_real_device_owner_and_commits_later() {
    fn store(code: &mut Vec<u8>, a: u16, v: u8) {
        code.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    }
    fn send(code: &mut Vec<u8>, v: u8) {
        store(code, 0xf0eb, v);
        code.extend([0x6a, 0x08, 0xf0, 0xe4, 0xe8, 8, 0x47, 0xf8]); // wait TEND
        code.extend([0x6a, 0x08, 0xf0, 0xe9]); // receive register, clear RDRF
    }
    let mut code = vec![0x79, 7, 0xff, 0x80];
    for (a, v) in [
        (0xfffb, 0x14),
        (0xf0e0, 0x8c),
        (0xf0e1, 0x40),
        (0xf0e2, 0x86),
        (0xf0e3, 0xc0),
        (0xffe4, 7),
        (0xffd4, 5),
        (0xf087, 8),
        (0xffec, 1),
        (0xffdc, 1),
    ] {
        store(&mut code, a, v);
    }
    store(&mut code, 0xffd4, 1);
    send(&mut code, 6);
    store(&mut code, 0xffd4, 5);
    store(&mut code, 0xffd4, 1);
    for v in [2, 0, 0x7e, 0xaa, 0xbb, 0xcc, 0xdd] {
        send(&mut code, v);
    }
    store(&mut code, 0xffd4, 5);
    code.extend([0x40, 0xfe]);
    let mut m = machine(&code);
    let mut events = Vec::new();
    m.run_until(Time::from_micros(1000), &[], &mut events)
        .unwrap();
    assert_eq!(m.eeprom()[2], 0xff, "unaddressed cells remain untouched");
    assert!(!events.iter().any(|e| matches!(e, Event::NvCommit { .. })));
    let snapshot = m.snapshot();
    let mut interrupted = state::restore_file(&snapshot);
    let partial = interrupted.eeprom();
    assert_ne!(partial[0x7e], 0xff, "erase has physically started");
    let mut loss = Vec::new();
    interrupted
        .run_until(
            Time::from_micros(7000),
            &[TimedInput {
                at: Time::from_micros(1000),
                input: Input::Power(false),
            }],
            &mut loss,
        )
        .unwrap();
    assert_eq!(interrupted.eeprom(), partial);
    assert!(!interrupted.powered());
    assert!(loss
        .iter()
        .any(|e| matches!(e, Event::NvInterrupted { .. })));
    assert!(!loss.iter().any(|e| matches!(e, Event::NvCommit { .. })));
    interrupted.power_on(&mut ()).unwrap();
    assert_eq!(interrupted.eeprom(), partial);
    let mut reset = state::restore_file(&snapshot);
    reset
        .run_until(
            Time::from_micros(7000),
            &[TimedInput {
                at: Time::from_micros(1000),
                input: Input::ResetPin(false),
            }],
            &mut (),
        )
        .unwrap();
    assert_eq!(&reset.eeprom()[0x7e..0x80], &[0xaa, 0xbb]);
    m.run_until(Time::from_micros(7000), &[], &mut events)
        .unwrap();
    assert_eq!(&m.eeprom()[0x7e..0x80], &[0xaa, 0xbb]);
    assert_eq!(&m.eeprom()[0..2], &[0xcc, 0xdd]);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::NvCommit { .. }))
            .count(),
        1
    );
}
#[test]
fn power_cycle_and_mcu_reset_are_distinct() {
    let mut m = machine(LOOP);
    m.run_until(Time::from_micros(20), &[], &mut ()).unwrap();
    assert_ne!(m.ram()[0], 0);
    let saved = m.eeprom().to_vec();
    let marker = m.ram()[0];
    m.power_off(&mut ()).unwrap();
    let retired = m.retired();
    m.run_until(Time::from_micros(100), &[], &mut ()).unwrap();
    assert_eq!(m.retired(), retired);
    assert!(!m.powered());
    m.power_on(&mut ()).unwrap();
    assert_eq!(m.ram()[0], marker);
    assert_eq!(m.eeprom().as_slice(), saved);
    m.run_until(Time::from_micros(400), &[], &mut ()).unwrap();
    assert_eq!(m.retired(), retired); // main crystal is still starting
    m.run_until(Time::from_micros(420), &[], &mut ()).unwrap();
    assert!(m.retired() > retired);
}

#[test]
fn all_nonvolatile_domains_can_be_reloaded_without_a_snapshot() {
    use hs_core::Conditions;
    let m = machine(LOOP);
    let mut sensor = m.sensor_nonvolatile();
    sensor[0x12 - 0x0b] = 0x5a; // BMA150 customer EEPROM working-image byte.
    let mut restored = Machine::with_persistent_state(
        Images {
            firmware: &*m.firmware(),
            eeprom: &m.eeprom(),
            eeprom_status: 0x84,
        },
        Conditions::default(),
        Some(&sensor),
    )
    .unwrap();
    assert_eq!(restored.sensor_nonvolatile(), sensor);
    assert_eq!(restored.eeprom_status(), 0x84);
    restored.power_off(&mut ()).unwrap();
    restored.power_on(&mut ()).unwrap();
    assert_eq!(restored.sensor_nonvolatile(), sensor);
    assert_eq!(restored.eeprom_status(), 0x84);
    assert!(Machine::with_persistent_state(
        Images {
            firmware: &*m.firmware(),
            eeprom: &m.eeprom(),
            eeprom_status: 0
        },
        Conditions::default(),
        Some(&sensor[..18]),
    )
    .is_err());
}

#[test]
fn zero_supply_stops_the_board_and_restoration_uses_the_power_domain() {
    let mut m = machine(LOOP);
    m.run_until(Time::from_micros(20), &[], &mut ()).unwrap();
    let retired = m.retired();
    let change = |us, mv| TimedInput {
        at: Time::from_micros(us),
        input: Input::SupplyMillivolts(mv),
    };
    m.run_until(Time::from_micros(100), &[change(20, 0)], &mut ())
        .unwrap();
    assert!(!m.powered());
    assert_eq!(m.retired(), retired);
    m.run_until(Time::from_micros(130), &[change(100, 3000)], &mut ())
        .unwrap();
    assert!(m.powered());
    assert!(m.retired() > 0);
    let mut off = Machine::with_conditions(
        Images {
            firmware: &*m.firmware(),
            eeprom: &m.eeprom(),
            eeprom_status: 0,
        },
        hs_core::Conditions {
            supply_millivolts: 0,
            ..Default::default()
        },
    )
    .unwrap();
    off.run_until(Time::from_micros(100), &[], &mut ()).unwrap();
    assert!(!off.powered());
    assert_eq!(off.retired(), 0);
}
