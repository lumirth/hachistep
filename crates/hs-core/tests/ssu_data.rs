//! Data holding registers remain independent of an in-flight serial byte.
#[path = "support/state.rs"]
mod state;
use hs_core::{Conditions, DigitalPin, Event, Images, Input, Machine, Time, TimedInput};

fn byte(code: &mut Vec<u8>, address: u16, value: u8) {
    code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
}
fn record(code: &mut Vec<u8>, address: u16, destination: u16) {
    code.extend([0x6a, 8, (address >> 8) as u8, address as u8]);
    code.extend([0x6a, 0x88, (destination >> 8) as u8, destination as u8]);
}
fn setup(code: &mut Vec<u8>, enable: u8) {
    for (address, value) in [
        (0xfffb, 0x14),
        (0xffd4, 7),
        (0xffe4, 7),
        (0xffdc, 1),
        (0xffec, 1),
        (0xf0e0, 0x8c),
        (0xf0e1, 0x40),
        (0xf0e2, 0x86),
        (0xf0e3, enable),
        (0xffd4, 6),
    ] {
        byte(code, address, value);
    }
}
fn machine(code: &[u8]) -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&0x100_u16.to_be_bytes());
    rom[68..70].copy_from_slice(&0x400_u16.to_be_bytes()); // SSU/IIC vector 34.
    rom[0x100..0x100 + code.len()].copy_from_slice(code);
    rom[0x400..0x40c].copy_from_slice(&[
        0xf9, 1, 0x6a, 0x89, 0xf8, 3, 0x6a, 9, 0xf0, 0xe9, 0x56, 0x70,
    ]);
    let mut conditions = Conditions::default();
    conditions.clocks.main_hz = 1_000_000;
    Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[255; 65536],
            eeprom_status: 0,
            sensor_nonvolatile: None,
        },
        conditions,
    )
    .unwrap()
}
fn partition(
    initial: Machine,
    end: Time,
    inputs: &[TimedInput],
    expected: &Machine,
    events: &[Event],
) {
    let mut split = initial;
    let mut observed = Vec::new();
    let mut cursor = 0;
    for quarter in 1..=1600 {
        let at = Time::from_raw(Time::from_micros(quarter).raw() / 4);
        if at > end {
            break;
        }
        cursor += split
            .run_until(at, &inputs[cursor..], &mut observed)
            .unwrap()
            .inputs_consumed;
        if quarter % 19 == 0 {
            split = state::restore_file(&split.snapshot());
        }
    }
    assert_eq!(observed, events);
    state::assert_same_state(&split, expected);
}

#[test]
fn a_queued_holding_byte_preserves_the_current_byte_and_flag_lifetimes() {
    let mut code = vec![];
    setup(&mut code, 0x80);
    byte(&mut code, 0xf0eb, 0x25);
    record(&mut code, 0xf0e4, 0xf800); // Load has emptied TDR; byte still active.
    byte(&mut code, 0xf0eb, 0x96); // Queue a different byte before completion.
    record(&mut code, 0xf0e4, 0xf801);
    for _ in 0..30 {
        code.extend([0, 0]);
    }
    record(&mut code, 0xf0e4, 0xf802);
    code.extend([0x40, 0xfe]);
    let initial = machine(&code);
    let mut whole = initial.clone();
    let end = Time::from_micros(400);
    let mut events = Vec::new();
    whole.run_until(end, &[], &mut events).unwrap();
    assert_eq!(&whole.ram()[0x80..0x83], &[4, 0, 12]);
    let writes: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::LcdWrite {
                at,
                page,
                column_byte,
                value,
            } => Some((*at, *page, *column_byte, *value)),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 2);
    assert_eq!((writes[0].1, writes[0].2, writes[0].3), (0, 0, 0x25));
    assert_eq!((writes[1].1, writes[1].2, writes[1].3), (0, 1, 0x96));
    // CKPH=0 samples on edge sixteen. If byte one finishes at SSCK tap n,
    // its next load is phi tick 2n+1 and byte two samples at tap n+16. The
    // intervening load fits between taps: terminal samples are 32 states apart.
    let duration = writes[1].0.raw() - writes[0].0.raw();
    assert!(
        duration.abs_diff(Time::from_micros(32).raw()) <= 1,
        "rational edge projection differs by at most one timestamp quantum: {:?}",
        writes
    );
    partition(initial, end, &[], &whole, &events);
}

#[test]
fn draining_retained_receive_data_removes_the_interrupt_before_unmasking() {
    for drain in [true, false] {
        let mut code = vec![0x79, 7, 0xff, 0x70];
        setup(&mut code, 0xc2); // Transmit/receive and receive-data interrupt.
        byte(&mut code, 0xf0eb, 0x25);
        for _ in 0..24 {
            code.extend([0, 0]);
        }
        record(&mut code, 0xf0e4, 0xf800);
        if drain {
            record(&mut code, 0xf0e9, 0xf801);
        }
        code.extend([0x06, 0x7f, 0, 0, 0x40, 0xfe]); // ANDC, deferred instruction, loop.
        let initial = machine(&code);
        let mut whole = initial.clone();
        let inputs = [TimedInput {
            at: Time::ZERO,
            input: Input::DigitalPin {
                pin: DigitalPin::P93,
                level: Some(true),
            },
        }];
        let mut events = Vec::new();
        let end = Time::from_micros(400);
        whole.run_until(end, &inputs, &mut events).unwrap();
        assert_eq!(whole.peek(0xf800).unwrap() & 2, 2);
        if drain {
            assert_eq!(whole.peek(0xf801).unwrap(), 255);
        }
        assert_eq!(whole.interrupt_entries(), u64::from(!drain));
        assert_eq!(whole.peek(0xf803).unwrap(), u8::from(!drain));
        partition(initial, end, &inputs, &whole, &events);
    }
}

#[test]
fn a_fault_after_a_holding_store_retains_the_preceding_partial_wire_history() {
    let mut code = vec![];
    setup(&mut code, 0x80);
    byte(&mut code, 0xf0eb, 0x25);
    byte(&mut code, 0xf0eb, 0x96);
    code.extend([0x57, 0xff]);
    let initial = machine(&code);
    let mut whole = initial.clone();
    let mut events = Vec::new();
    let error = whole
        .run_until(Time::from_micros(400), &[], &mut events)
        .unwrap_err();
    assert!(matches!(error, hs_core::Error::Decode { .. }));
    assert_eq!(whole.peek(0xf0eb).unwrap(), 0x96);
    assert_eq!(whole.peek(0xf0e4).unwrap() & 12, 0);
    // The partial first frame has launched two zero bits of 0x25. The queued
    // 0x96 starts with one, but cannot replace the byte already in the shifter.
    assert_eq!(whole.peek(0xf0e0).unwrap() & 0x10, 0);
    assert!(!events.iter().any(|e| matches!(e, Event::LcdWrite { .. })));
    let mut split = initial;
    let mut observed = Vec::new();
    let mut split_error = None;
    for quarter in 1..=1600 {
        let at = Time::from_raw(Time::from_micros(quarter).raw() / 4);
        if let Err(error) = split.run_until(at, &[], &mut observed) {
            split_error = Some(error);
            break;
        }
        if quarter % 19 == 0 {
            split = state::restore_file(&split.snapshot());
        }
    }
    assert_eq!(split_error, Some(error));
    assert_eq!(events, observed);
    state::assert_same_state(&whole, &split);
    state::assert_same_state(&whole, &state::restore_file(&whole.snapshot()));
}
