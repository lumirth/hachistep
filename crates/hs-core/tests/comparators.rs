//! Register/latch expectations from REJ09B0152-0300 §18. Response times below
//! are explicit simulation parameters, not new physical captures.
use hs_core::{
    mcu::{comparators::Comparators, Mcu},
    signals::AnalogPin,
    Images, Input, Machine, Time, TimedInput,
};
fn at(us: u64) -> Time {
    Time::from_micros(us)
}
fn settled(c: &mut Comparators, us: u64) {
    c.sync(at(us)).unwrap();
}
fn init(value: u8, mv: u16) -> Comparators {
    let mut c = Comparators::default();
    c.set_inputs(at(0), 3000, 1500, [mv, 0]).unwrap();
    c.set_gate(true, at(0)).unwrap();
    c.write(0xf0dc, value, at(0)).unwrap();
    settled(&mut c, 20);
    c
}

#[test]
fn threshold_ladder_covers_all_sixteen_selections() {
    for selection in 0..16u8 {
        let threshold = (11 + u16::from(selection)) * 100;
        let mut c = init(0x80 | selection, threshold);
        assert_eq!(c.peek(0xf0de) & 1, 0, "equal threshold must be low");
        c.set_inputs(at(21), 3000, 0, [threshold + 1, 0]).unwrap();
        settled(&mut c, 40);
        assert_eq!(c.peek(0xf0de) & 1, 1, "selection {selection}");
    }
}
#[test]
fn hysteresis_keeps_history_in_the_deadband() {
    let mut c = init(0x98, 2000); // 1900 mV upper, 1700 mV lower at 3 V.
    assert_eq!(c.peek(0xf0de) & 1, 1);
    for (time, mv, expected) in [
        (21, 1800, 1),
        (41, 1700, 0),
        (61, 1800, 0),
        (81, 1900, 0),
        (101, 1901, 1),
    ] {
        c.set_inputs(at(time), 3000, 0, [mv, 0]).unwrap();
        settled(&mut c, time + 19);
        assert_eq!(c.peek(0xf0de) & 1, expected);
    }
}
#[test]
fn interrupts_are_read_armed_not_unconditionally_edge_triggered() {
    let mut c = init(0xc8, 2000);
    assert!(!c.interrupt(), "CME+CMIE alone must not arm the latch");
    c.read(0xf0de);
    c.set_inputs(at(21), 3000, 0, [1000, 0]).unwrap();
    settled(&mut c, 40);
    assert!(c.interrupt());
    c.write(0xf0de, 0, at(40)).unwrap(); // No read of CMF=1 yet.
    assert!(c.interrupt());
    assert_eq!(c.read(0xf0de) & 0x11, 0x10);
    c.write(0xf0de, 0, at(40)).unwrap();
    assert!(!c.interrupt());
    c.set_inputs(at(41), 3000, 0, [2000, 0]).unwrap();
    settled(&mut c, 60);
    assert!(
        c.interrupt(),
        "read-to-clear also rearms at the new CDR baseline"
    );
}
#[test]
fn same_time_read_masks_new_interrupt_but_keeps_older_flag() {
    let mut c = init(0xc8, 0);
    c.read(0xf0de);
    c.set_inputs(at(21), 3000, 0, [2000, 0]).unwrap();
    let due = c.deadline().unwrap();
    c.sync(due).unwrap();
    assert_eq!(c.read(0xf0de) & 0x11, 1);
    assert!(!c.interrupt());
    c.set_inputs(at(50), 3000, 0, [0, 0]).unwrap();
    settled(&mut c, 70);
    assert!(c.interrupt());
    c.set_inputs(at(71), 3000, 0, [2000, 0]).unwrap();
    c.sync(c.deadline().unwrap()).unwrap();
    assert_eq!(c.read(0xf0de) & 0x10, 0x10);
    c.write(0xf0de, 0, at(86)).unwrap();
    assert_eq!(c.read(0xf0de) & 0x10, 0);
}
#[test]
fn independent_channels_external_reference_and_interrupt_vector() {
    let mut m = Mcu::new(&vec![0; 49152], Default::default()).unwrap();
    m.comparators
        .set_inputs(at(0), 3000, 1500, [0, 1600])
        .unwrap();
    m.write8(0xfffb, 6, true, at(0), &mut ()).unwrap();
    m.write8(0xf0dd, 0xe0, true, at(0), &mut ()).unwrap();
    m.sync(at(20)).unwrap();
    assert_eq!(m.read8(0xf0de, at(20)).unwrap() & 3, 2);
    m.comparators
        .set_inputs(at(21), 3000, 1700, [0, 1600])
        .unwrap();
    m.sync(at(40)).unwrap();
    assert_eq!(m.interrupt(), Some(36));
    assert_eq!(m.read8(0xf0de, at(40)).unwrap() & 0x30, 0x20);
    m.write8(0xf0de, 0x10, true, at(40), &mut ()).unwrap();
    assert_eq!(m.interrupt(), None);
    assert!(m.write8(0xf0dc, 0xb0, true, at(40), &mut ()).is_err());
}
#[test]
fn module_stop_is_distinct_from_standby_and_reset() {
    let mut c = init(0xc8, 0);
    c.read(0xf0de);
    assert!(
        c.set_gate(false, at(20)).is_err(),
        "§18.5 requires CME clear first"
    );
    c.write(0xf0dc, 0, at(20)).unwrap();
    c.set_gate(false, at(20)).unwrap();
    assert_eq!(c.deadline(), None);
    c.reset(at(30));
    assert_eq!(c.peek(0xf0dc), 0);
    assert_eq!(c.peek(0xf0de), 0);
}

fn fixture() -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
    rom[72..74].copy_from_slice(&0x200u16.to_be_bytes()); // Comparator vector 36.
    let mut code = vec![
        0x7a, 0x07, 0, 0, 0xff, 0x70, 0xf8, 6, 0x38, 0xfb, 0xf8, 0xc8, 0x6a, 0x88, 0xf0, 0xdc,
    ];
    for _ in 0..64 {
        code.extend([0, 0]);
    }
    code.extend([0x6a, 0x08, 0xf0, 0xde, 0x06, 0x7f, 0x01, 0x80, 0x40, 0xfc]);
    rom[0x100..0x100 + code.len()].copy_from_slice(&code);
    // Mark RAM, read/clear/rearm comparator flag, return to sleep loop.
    rom[0x200..0x214].copy_from_slice(&[
        0xf8, 1, 0x6a, 0x88, 0xf7, 0x80, 0x6a, 0x08, 0xf0, 0xde, 0xf8, 0, 0x6a, 0x88, 0xf0, 0xde,
        0x56, 0x70, 0, 0,
    ]);
    Machine::new(Images {
        firmware: &rom,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
    })
    .unwrap()
}
#[test]
fn guest_program_wakes_on_real_comparator_vector_and_replays_exactly() {
    let inputs = [
        TimedInput {
            at: at(0),
            input: Input::AnalogPin {
                pin: AnalogPin::Pb4,
                millivolts: Some(1000),
            },
        },
        TimedInput {
            at: at(200),
            input: Input::AnalogPin {
                pin: AnalogPin::Pb4,
                millivolts: Some(2100),
            },
        },
    ];
    let mut a = fixture();
    let mut b = a.clone();
    let (mut x, mut y) = (Vec::new(), Vec::new());
    a.run_until(at(1000), &inputs, &mut x).unwrap();
    let mut cursor = 0;
    for us in 1..=1000 {
        cursor += b
            .run_until(at(us), &inputs[cursor..], &mut y)
            .unwrap()
            .inputs_consumed;
        if us == 208 {
            b = Machine::from_snapshot(&b.snapshot());
        }
    }
    assert_eq!(a.peek(0xf780).unwrap(), 1);
    assert_eq!(a.interrupt_entries(), 1);
    assert_eq!(a, b);
    assert_eq!(x, y);
}
#[test]
fn analog_input_collisions_are_checked_before_machine_mutation() {
    let mut a = fixture();
    let before = a.clone();
    let input = TimedInput {
        at: at(0),
        input: Input::AnalogPin {
            pin: AnalogPin::Pb4,
            millivolts: Some(1000),
        },
    };
    assert!(a.run_until(at(10), &[input, input], &mut ()).is_err());
    assert_eq!(a, before);
}

#[test]
fn inspection_after_explicit_power_on_does_not_project_before_reset() {
    let mut a = fixture();
    a.run_until(at(1000), &[], &mut ()).unwrap();
    a.power_off(&mut ()).unwrap();
    a.power_on(&mut ()).unwrap();
    let before = a.clone();
    assert_eq!(a.peek(0xf0dc).unwrap(), 0);
    assert_eq!(a, before);
}

#[test]
fn vcref_is_p30_not_the_p32_transmit_pin() {
    // REJ09B0152-0300 §1.3 and §8.2: P30/SCK3/VCref, P32/TXD3/IrTXD.
    let mut g = hs_core::mcu::gpio::Gpio::default();
    g.write(0xffc2, 1).unwrap();
    g.write(0xffe6, 4).unwrap(); // P32 output, leave P30 input
    g.write(0xffd6, 4).unwrap(); // preserve high transmitter output
    let mut levels = [None; 7];
    levels[6] = Some(false);
    g.set_analog_levels(levels);
    g.resolve(Default::default(), 0, 0, None);
    assert_eq!(g.read(0xffd6) & 5, 4);
    levels[6] = Some(true);
    g.set_analog_levels(levels);
    g.resolve(Default::default(), 0, 0, None);
    assert_eq!(g.read(0xffd6) & 5, 5);
}
