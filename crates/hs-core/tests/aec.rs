//! Independently expressed register/count/PWM expectations from MCU §13.
//! Tests establish documented digital rules and the explicitly chosen
//! reference-edge interpretation, not new physical measurements.
use hs_core::{
    cpu::WriteOrigin,
    mcu::{
        aec::Aec,
        clocks::{Clocks, Frequencies, Tap},
        control::Mode,
        Mcu,
    },
    DigitalPin, Images, Input, Machine, Time, TimedInput,
};
fn clocks() -> Clocks {
    Clocks::new(
        Time::ZERO,
        Frequencies {
            main_hz: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap()
}
fn t(c: &Clocks, n: u64) -> Time {
    c.edge(n, Tap::system(1)).unwrap()
}
fn make(c: &Clocks) -> Aec {
    let mut a = Aec::default();
    a.set_power(true, true, true, true, Time::ZERO, c).unwrap();
    a.input_pins([Some(false), Some(false), Some(true)], Time::ZERO, c)
        .unwrap();
    a
}
fn count(a: &Aec) -> u16 {
    u16::from_be_bytes([a.peek(0xff96), a.peek(0xff97)])
}
#[test]
fn reset_readonly_and_undefined_register_contracts() {
    let c = clocks();
    let mut a = make(&c);
    assert_eq!(a.read_word(0xff8c).unwrap(), 0xffff);
    assert_eq!(a.read_word(0xff8e).unwrap(), 0);
    for addr in [0xff92, 0xff94, 0xff95, 0xff96, 0xff97] {
        assert_eq!(a.read(addr), 0);
    }
    a.write(0xff96, 255, Time::ZERO, &c).unwrap();
    a.write(0xff97, 255, Time::ZERO, &c).unwrap();
    assert_eq!(count(&a), 0);
}
#[test]
fn sixteen_bit_counter_cascades_and_only_high_overflow_sets_status() {
    let c = clocks();
    let mut a = make(&c);
    a.write(0xff94, 0x10, Time::ZERO, &c).unwrap();
    a.write(0xff95, 0x0f, Time::ZERO, &c).unwrap();
    assert_eq!(a.deadline(&c).unwrap(), Some(t(&c, 131072)));
    for (edges, value, flag) in [
        (510, 255, 0),
        (512, 256, 0),
        (131070, 65535, 0),
        (131072, 0, 0x80),
        (131074, 1, 0x80),
    ] {
        a.sync(t(&c, edges), &c).unwrap();
        assert_eq!(count(&a), value);
        assert_eq!(a.peek(0xff95) & 0xc0, flag);
    }
    assert_eq!(a.take_requests(), 1);
    assert_eq!(a.take_requests(), 0);
}
#[test]
fn independent_eight_bit_sources_and_qualification() {
    let c = clocks();
    let mut a = make(&c);
    a.write(0xff94, 0x60, Time::ZERO, &c).unwrap(); // H /2, L /4.
    a.write(0xff95, 0x1f, Time::ZERO, &c).unwrap();
    a.sync(t(&c, 512), &c).unwrap();
    assert_eq!(count(&a), 128);
    assert_eq!(a.peek(0xff95) & 0xc0, 0x80);
    a.write(0xff95, 0x1f, t(&c, 512), &c).unwrap();
    assert_eq!(
        a.peek(0xff95) & 0xc0,
        0x80,
        "read-one qualification required"
    );
    a.read(0xff95);
    a.write(0xff95, 0x1f, t(&c, 512), &c).unwrap();
    assert_eq!(a.peek(0xff95) & 0xc0, 0);
    a.sync(t(&c, 1024), &c).unwrap();
    assert_eq!(a.peek(0xff95) & 0xc0, 0xc0);
    a.read(0xff95);
    a.write(0xff95, 0x9f, t(&c, 1024), &c).unwrap();
    assert_eq!(a.peek(0xff95) & 0xc0, 0x80);
}
#[test]
fn disabling_count_retains_value_and_reset_control_clears_it() {
    let c = clocks();
    let mut a = make(&c);
    a.write(0xff94, 0x10, Time::ZERO, &c).unwrap();
    a.write(0xff95, 0x0f, Time::ZERO, &c).unwrap();
    a.sync(t(&c, 600), &c).unwrap();
    assert_eq!(count(&a), 300);
    a.write(0xff95, 0x0b, t(&c, 600), &c).unwrap();
    a.sync(t(&c, 900), &c).unwrap();
    assert_eq!(count(&a), 300);
    a.write(0xff95, 0, t(&c, 900), &c).unwrap();
    assert_eq!(count(&a), 0);
    a.write(0xff95, 2, t(&c, 900), &c).unwrap();
    assert_eq!(count(&a), 0);
    a.write(0xff95, 0x0f, t(&c, 900), &c).unwrap();
    a.sync(t(&c, 1000), &c).unwrap();
    a.write(0xff95, 7, t(&c, 1000), &c).unwrap();
    assert_eq!(count(&a), 50);
    a.sync(t(&c, 1412), &c).unwrap();
    assert_eq!(
        count(&a),
        0,
        "disabled high counter does not receive low overflow"
    );
}
#[test]
fn asynchronous_selected_edges_work_without_system_clock() {
    let c = clocks();
    for mode in 0..3u8 {
        let mut a = make(&c);
        a.write(0xff92, mode << 4, Time::ZERO, &c).unwrap();
        a.write(0xff95, 0x0f, Time::ZERO, &c).unwrap();
        a.set_power(true, false, false, false, Time::ZERO, &c)
            .unwrap();
        for n in 1..=20 {
            a.input_pins([Some(false), Some(n & 1 != 0), Some(true)], t(&c, n), &c)
                .unwrap();
        }
        assert_eq!(count(&a), if mode == 2 { 20 } else { 10 });
        assert_eq!(a.deadline(&c).unwrap(), None);
    }
}
#[test]
fn gate_return_can_create_an_additional_count_as_in_figure_13_5() {
    let c = clocks();
    let mut a = make(&c);
    a.write(0xff92, 0x28, Time::ZERO, &c).unwrap(); // L both edges, gate both edges.
    a.write(0xff95, 0x0f, Time::ZERO, &c).unwrap();
    let sequence = [
        (true, true, 1),
        (false, true, 2),
        (false, false, 2),
        (true, false, 2),
        (false, false, 2),
        (true, false, 2),
        (true, true, 3),
        (false, true, 4),
    ];
    for (i, (pin, gate, expected)) in sequence.into_iter().enumerate() {
        a.input_pins(
            [Some(false), Some(pin), Some(gate)],
            t(&c, i as u64 + 1),
            &c,
        )
        .unwrap();
        assert_eq!(count(&a), expected);
    }
    assert_eq!(a.take_requests(), 2);
}
#[test]
fn stopped_counters_do_not_disable_independent_gate_interrupt() {
    let c = clocks();
    let mut a = make(&c);
    a.input_pins([Some(false), Some(false), Some(false)], t(&c, 3), &c)
        .unwrap();
    assert_eq!(a.take_requests(), 2);
    assert_eq!(count(&a), 0);
    a.input_pins([Some(false), Some(false), Some(true)], t(&c, 4), &c)
        .unwrap();
    assert_eq!(a.take_requests(), 0);
}
#[test]
fn pwm_low_high_period_and_invalid_duty_are_distinct() {
    let c = clocks();
    let mut a = make(&c);
    a.write_word(0xff8c, 3, Time::ZERO, &c).unwrap();
    a.write_word(0xff8e, 1, Time::ZERO, &c).unwrap();
    a.write(0xff92, 0x0a, Time::ZERO, &c).unwrap();
    assert_eq!(
        a.take_requests(),
        2,
        "selecting initial low PWM lowers the old high gate"
    );
    for (edge, high, request) in [
        (1, false, 0),
        (3, false, 0),
        (4, true, 2),
        (7, true, 0),
        (8, false, 2),
        (12, true, 2),
    ] {
        a.sync(t(&c, edge), &c).unwrap();
        assert_eq!(a.pwm_output(), Some(high));
        assert_eq!(a.take_requests(), request);
    }
    a.write(0xff92, 0, t(&c, 12), &c).unwrap();
    a.write_word(0xff8e, 3, t(&c, 12), &c).unwrap();
    a.write(0xff92, 2, t(&c, 12), &c).unwrap();
    a.sync(t(&c, 10000), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(false));
    assert_eq!(a.deadline(&c).unwrap(), None);
}
#[test]
fn pwm_gating_retains_phase_and_shared_divider() {
    let c = clocks();
    let mut a = make(&c);
    a.write_word(0xff8c, 3, Time::ZERO, &c).unwrap();
    a.write_word(0xff8e, 1, Time::ZERO, &c).unwrap();
    a.write(0xff92, 2, Time::ZERO, &c).unwrap();
    a.set_power(true, false, false, false, t(&c, 3), &c)
        .unwrap();
    assert_eq!(a.deadline(&c).unwrap(), None);
    a.set_power(true, true, true, true, t(&c, 101), &c).unwrap();
    assert_eq!(a.deadline(&c).unwrap(), Some(t(&c, 102)));
    a.sync(t(&c, 102), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(true));
}
#[test]
fn watch_pwm_and_external_inputs_have_different_power_domains() {
    let mut m = Mcu::new(&vec![0; 49152], Default::default()).unwrap();
    m.write8(0xfffb, 12, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    m.write8(0xff94, 12, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    m.write16(0xff8c, 3, Time::ZERO).unwrap();
    m.write16(0xff8e, 1, Time::ZERO).unwrap();
    m.write8(0xff92, 2, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    for (mode, stabilizing, clock, pad) in [
        (Mode::Active, None, true, true),
        (Mode::Watch, None, true, true),
        (Mode::Subsleep, None, true, true),
        (Mode::Standby, None, false, false),
        (Mode::Active, Some(Mode::Standby), false, false),
        (Mode::Active, Some(Mode::Watch), true, true),
    ] {
        m.control.mode = mode;
        m.control.stabilizing_from = stabilizing;
        m.apply_gates(Time::ZERO, &mut ()).unwrap();
        assert_eq!(m.aec.deadline(&m.clocks).unwrap().is_some(), clock);
        assert_eq!(m.aec.pwm_output().is_some(), pad);
    }
}
#[test]
fn controller_request_is_separate_from_counter_status() {
    let mut m = Mcu::new(&vec![0; 49152], Default::default()).unwrap();
    let c = m.clocks.clone();
    m.write8(0xfffb, 12, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    m.aec
        .input_pins([None, None, Some(true)], Time::ZERO, &c)
        .unwrap();
    m.write8(0xff94, 0x10, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    m.write8(0xff95, 0x17, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    m.control.ien2 = 1;
    let at = c.edge(512, Tap::system(1)).unwrap();
    m.sync(at).unwrap();
    assert_eq!(m.interrupt(), Some(32));
    m.write8(0xfff7, 0, WriteOrigin::MovByte, at, &mut ())
        .unwrap();
    assert_eq!(m.interrupt(), None);
    assert_ne!(m.aec.peek(0xff95) & 0x40, 0);
    m.sync(c.edge(1024, Tap::system(1)).unwrap()).unwrap();
    assert_eq!(m.interrupt(), Some(32));
}
#[test]
fn pwm_and_counter_evolution_is_partition_invariant() {
    let c = clocks();
    let mut a = make(&c);
    a.write(0xff94, 0x60, Time::ZERO, &c).unwrap();
    a.write(0xff95, 0x1f, Time::ZERO, &c).unwrap();
    a.write_word(0xff8c, 23, Time::ZERO, &c).unwrap();
    a.write_word(0xff8e, 7, Time::ZERO, &c).unwrap();
    a.write(0xff92, 0x0a, Time::ZERO, &c).unwrap();
    let mut b = a.clone();
    a.sync(t(&c, 50000), &c).unwrap();
    for n in 1..=50000 {
        b.sync(t(&c, n), &c).unwrap();
    }
    assert_eq!(a, b);
}
#[test]
fn reserved_fields_and_disconnected_selectors_retain_register_values() {
    let c = clocks();
    let mut a = make(&c);
    for (address, value) in [(0xff92, 0xfd), (0xff94, 0x0f), (0xff95, 0x3f)] {
        a.write(address, value, Time::ZERO, &c).unwrap();
        assert_eq!(a.read(address), value);
    }
    a.input_pins([Some(true); 3], t(&c, 10), &c).unwrap();
    a.input_pins([Some(false); 3], t(&c, 20), &c).unwrap();
    assert_eq!(count(&a), 0);
    assert_eq!(a.take_requests(), 0);
    a.write(0xff92, 0xff, t(&c, 20), &c).unwrap();
    a.sync(t(&c, 10000), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(false));
    assert_eq!(a.deadline(&c).unwrap(), None);
}
#[test]
fn live_pwm_writes_keep_counter_phase_and_force_low_through_the_gate() {
    let c = clocks();
    let mut a = make(&c);
    a.write_word(0xff8c, 9, Time::ZERO, &c).unwrap();
    a.write_word(0xff8e, 1, Time::ZERO, &c).unwrap();
    a.write(0xff92, 0x0a, Time::ZERO, &c).unwrap();
    a.sync(t(&c, 6), &c).unwrap(); // PWM count3, high.
    a.take_requests();
    a.write_word(0xff8e, 9, t(&c, 6), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(false));
    assert_eq!(a.take_requests(), 2);
    a.sync(t(&c, 10), &c).unwrap(); // Count5 despite output forced low.
    a.write_word(0xff8e, 7, t(&c, 10), &c).unwrap();
    assert_eq!(a.deadline(&c).unwrap(), Some(t(&c, 16)));
    a.sync(t(&c, 16), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(true));
    a.write(0xff94, 14, t(&c, 17), &c).unwrap(); // Park the selected clock.
    a.sync(t(&c, 100), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(true));
    assert_eq!(a.deadline(&c).unwrap(), None);
    a.write(0xff94, 2, t(&c, 100), &c).unwrap(); // phi/4, count8 remains.
    assert_eq!(a.deadline(&c).unwrap(), Some(t(&c, 108)));
    a.sync(t(&c, 108), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(false));
}
#[test]
fn live_period_below_count_waits_for_wrapping_equality() {
    let c = clocks();
    let mut a = make(&c);
    a.write_word(0xff8c, 9, Time::ZERO, &c).unwrap();
    a.write_word(0xff8e, 1, Time::ZERO, &c).unwrap();
    a.write(0xff92, 2, Time::ZERO, &c).unwrap();
    a.sync(t(&c, 6), &c).unwrap(); // Count3, already past the new period2.
    a.write_word(0xff8c, 2, t(&c, 6), &c).unwrap();
    assert_eq!(a.deadline(&c).unwrap(), Some(t(&c, 131078)));
    a.sync(t(&c, 131078), &c).unwrap();
    assert_eq!(a.pwm_output(), Some(false));
}
#[test]
fn live_counter_clock_selection_can_create_a_rising_edge() {
    let c = clocks();
    let mut a = make(&c);
    a.write(0xff94, 0x80, Time::ZERO, &c).unwrap(); // H phi/4.
    a.write(0xff95, 0x1f, Time::ZERO, &c).unwrap();
    a.write(0xff94, 0x40, t(&c, 2), &c).unwrap(); // Low phi/4 -> high phi/2.
    assert_eq!(a.read(0xff96), 1);
    a.sync(t(&c, 4), &c).unwrap();
    assert_eq!(a.read(0xff96), 2);
}
fn fixture(gate_irq: bool) -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
    let vector = if gate_irq { 18 } else { 32 };
    rom[vector * 2..vector * 2 + 2].copy_from_slice(&0x200u16.to_be_bytes());
    let mut code = vec![0x7a, 0x07, 0, 0, 0xff, 0x70];
    for (address, value) in [
        (0xfffb, 12),
        (0xffc0, 8),
        (0xffc0, 0x28),
        (0xff92, 0x10),
        (0xff95, 0x17),
        (
            if gate_irq { 0xfff3 } else { 0xfff4 },
            if gate_irq { 4 } else { 1 },
        ),
    ] {
        code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
    }
    code.extend([0x06, 0x7f, 0x01, 0x80, 0x40, 0xfc]);
    rom[0x100..0x100 + code.len()].copy_from_slice(&code);
    let h = [
        0xf8,
        1,
        0x6a,
        0x88,
        0xf7,
        0x80,
        0x28,
        0x95,
        0xf8,
        0x17,
        0x38,
        0x95,
        0xf8,
        0,
        0x38,
        if gate_irq { 0xf6 } else { 0xf7 },
        0x56,
        0x70,
    ];
    rom[0x200..0x200 + h.len()].copy_from_slice(&h);
    Machine::new(Images {
        firmware: &rom,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
    })
    .unwrap()
}
fn pin(us: u64, pin: DigitalPin, high: bool) -> TimedInput {
    TimedInput {
        at: Time::from_micros(us),
        input: Input::DigitalPin {
            pin,
            level: Some(high),
        },
    }
}
#[test]
fn guest_external_count_overflow_vectors_32_and_replays() {
    let mut inputs = vec![
        pin(0, DigitalPin::P11, false),
        pin(0, DigitalPin::P12, true),
    ];
    for i in 0..256 {
        inputs.push(pin(100 + i * 10, DigitalPin::P11, true));
        inputs.push(pin(105 + i * 10, DigitalPin::P11, false));
    }
    let mut a = fixture(false);
    let mut b = a.clone();
    let (mut x, mut y) = (Vec::new(), Vec::new());
    a.run_until(Time::from_micros(3000), &inputs, &mut x)
        .unwrap();
    let mut cursor = 0;
    for us in 1..=3000 {
        cursor += b
            .run_until(Time::from_micros(us), &inputs[cursor..], &mut y)
            .unwrap()
            .inputs_consumed;
        if us == 1337 {
            b = Machine::from_snapshot(&b.snapshot());
        }
    }
    assert_eq!(a.peek(0xf780).unwrap(), 1);
    assert_eq!(a.interrupt_entries(), 1);
    assert_eq!(a, b);
    assert_eq!(x, y);
}
#[test]
fn guest_gate_input_alone_vectors_18() {
    let mut a = fixture(true);
    let inputs = [
        pin(0, DigitalPin::P11, false),
        pin(0, DigitalPin::P12, true),
        pin(100, DigitalPin::P12, false),
    ];
    a.run_until(Time::from_micros(300), &inputs, &mut ())
        .unwrap();
    assert_eq!(a.peek(0xf780).unwrap(), 1);
    assert_eq!(a.interrupt_entries(), 1);
}
#[test]
fn pwm_output_pin_route_and_digital_collision_validation() {
    let mut m = fixture(false);
    // The routing seam resolves physical P12, not a direct EEPROM-byte callback.
    let mut g = hs_core::mcu::gpio::Gpio::default();
    g.write(0xffc0, 0x20).unwrap();
    g.set_aec_output(true, Some(false));
    assert!(
        g.resolve(Default::default(), 0, 0, [None; 2])
            .eeprom_selected
    );
    g.set_aec_output(true, Some(true));
    assert!(
        !g.resolve(Default::default(), 0, 0, [None; 2])
            .eeprom_selected
    );
    // Same-node contradictions are rejected before run/input mutation.
    let change = pin(0, DigitalPin::P12, true);
    let before = m.clone();
    assert!(m
        .run_until(Time::from_micros(10), &[change, change], &mut ())
        .is_err());
    assert_eq!(m, before);
}

#[test]
fn inactive_module_does_not_replay_elapsed_clocks_when_reenabled() {
    let mut c = clocks();
    let mut a = Aec::default();
    a.write(0xff94, 0x10, Time::ZERO, &c).unwrap(); // L: phi/2
    a.write(0xff95, 0x17, Time::ZERO, &c).unwrap();
    let pins = [Some(false), Some(false), Some(true)];
    a.input_pins(pins, Time::ZERO, &c).unwrap();
    let end = t(&c, 20000);
    a.sync(end, &c).unwrap();
    c.select_system(end, hs_core::mcu::clocks::Source::Watch, 1)
        .unwrap();
    a.set_power(true, true, true, true, end, &c).unwrap();
    assert_eq!(a.peek(0xff97), 0);
    let next = c
        .after(end, 2, hs_core::mcu::clocks::Tap::system(1))
        .unwrap();
    a.sync(next, &c).unwrap();
    assert_eq!(a.peek(0xff97), 1);
    assert_eq!(a.take_requests(), 0);
}
