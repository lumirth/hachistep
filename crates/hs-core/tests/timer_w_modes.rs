//! Independent cases for the Timer W buffer and contention rules in §10.7.
use hs_core::{
    diagnostic::cpu::WriteOrigin,
    diagnostic::mcu::{
        clocks::{Clocks, Frequencies, Tap},
        control::Mode,
        timer_w::TimerW,
        Mcu,
    },
    Time,
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
fn edge(c: &Clocks, n: u64) -> Time {
    c.edge(n, Tap::system(1)).unwrap()
}
fn timer(c: &Clocks) -> TimerW {
    let mut w = TimerW::default();
    w.set_gate(true, Time::ZERO, c).unwrap();
    w
}

#[test]
fn buffered_compare_uses_old_value_then_loads_buffer() {
    let c = clocks();
    let mut w = timer(&c);
    w.write_word(0xf0f8, 2, Time::ZERO, &c).unwrap();
    w.write_word(0xf0fc, 5, Time::ZERO, &c).unwrap();
    w.write(0xf0f1, 0x80, Time::ZERO, &c).unwrap();
    w.write(0xf0f0, 0x90, Time::ZERO, &c).unwrap();
    w.sync(edge(&c, 2), &c).unwrap();
    assert_eq!(w.word(0xf0f8), 2);
    w.sync(edge(&c, 3), &c).unwrap();
    assert_eq!(w.word(0xf0f8), 5);
    assert_eq!(w.word(0xf0f6), 0);
    w.sync(edge(&c, 8), &c).unwrap();
    assert_eq!(w.word(0xf0f6), 5);
    w.sync(edge(&c, 9), &c).unwrap();
    assert_eq!(w.word(0xf0f6), 0);
}
#[test]
fn buffer_register_can_still_produce_its_own_compare_output() {
    let c = clocks();
    let mut w = timer(&c);
    w.write_word(0xf0f8, 2, Time::ZERO, &c).unwrap();
    w.write_word(0xf0fc, 5, Time::ZERO, &c).unwrap();
    w.write(0xf0f5, 0x8b, Time::ZERO, &c).unwrap(); // C toggle.
    w.write(0xf0f0, 0x90, Time::ZERO, &c).unwrap();
    w.sync(edge(&c, 6), &c).unwrap();
    assert_eq!(w.outputs() & 4, 4);
    assert_eq!(w.peek(0xf0f3) & 5, 5);
}
#[test]
fn cpu_writes_win_buffer_collisions_but_transfer_samples_old_buffer() {
    let c = clocks();
    for write_active in [false, true] {
        let mut w = timer(&c);
        w.write_word(0xf0f8, 2, Time::ZERO, &c).unwrap();
        w.write_word(0xf0fc, 7, Time::ZERO, &c).unwrap();
        w.write(0xf0f4, 0x8b, Time::ZERO, &c).unwrap();
        w.write(0xf0f0, 0x90, Time::ZERO, &c).unwrap();
        w.write_word(
            if write_active { 0xf0f8 } else { 0xf0fc },
            99,
            edge(&c, 3),
            &c,
        )
        .unwrap();
        assert_eq!(w.word(0xf0f8), if write_active { 99 } else { 7 });
        assert_eq!(w.word(0xf0fc), if write_active { 7 } else { 99 });
        assert_eq!(w.outputs() & 1, 1);
        assert_eq!(w.peek(0xf0f3) & 1, 1);
    }
}
#[test]
fn counter_clear_beats_write_but_plain_increment_does_not() {
    let c = clocks();
    for clear in [false, true] {
        let mut w = timer(&c);
        w.write_word(0xf0f8, 2, Time::ZERO, &c).unwrap();
        w.write(0xf0f1, if clear { 0x80 } else { 0 }, Time::ZERO, &c)
            .unwrap();
        w.write(0xf0f0, 0x80, Time::ZERO, &c).unwrap();
        w.write_word(0xf0f6, 1234, edge(&c, 3), &c).unwrap();
        assert_eq!(w.word(0xf0f6), if clear { 0 } else { 1234 });
    }
}
#[test]
fn simultaneous_pwm_period_and_duty_matches_retain_current_output() {
    let c = clocks();
    let mut w = timer(&c);
    w.write_word(0xf0f8, 5, Time::ZERO, &c).unwrap();
    w.write_word(0xf0fa, 2, Time::ZERO, &c).unwrap();
    w.write(0xf0f1, 0x82, Time::ZERO, &c).unwrap();
    w.write(0xf0f0, 0x81, Time::ZERO, &c).unwrap();
    w.sync(edge(&c, 3), &c).unwrap();
    assert_eq!(w.outputs() & 2, 0);
    w.write_word(0xf0fa, 5, edge(&c, 4), &c).unwrap();
    w.sync(edge(&c, 6), &c).unwrap();
    assert_eq!(
        w.outputs() & 2,
        0,
        "equal matches must not force programmed initial level"
    );
    assert_eq!(w.peek(0xf0f3) & 3, 3);
}
fn capture(c: &Clocks, running: bool) -> TimerW {
    let mut w = timer(c);
    w.write_word(0xf0f6, if running { 0 } else { 123 }, Time::ZERO, c)
        .unwrap();
    w.write_word(0xf0f8, 0x1234, Time::ZERO, c).unwrap();
    w.write(0xf0f4, 0x8e, Time::ZERO, c).unwrap(); // Capture A, both edges.
    w.write(0xf0f0, if running { 0x90 } else { 0x10 }, Time::ZERO, c)
        .unwrap();
    w.input_pins([Some(false), None, None, None, None], Time::ZERO, c)
        .unwrap();
    w.input_pins([Some(true), None, None, None, None], edge(c, 3), c)
        .unwrap();
    w
}
#[test]
fn capture_buffers_old_value_and_does_not_create_compare_flags() {
    let c = clocks();
    let mut w = capture(&c, true);
    w.sync(edge(&c, 5), &c).unwrap();
    assert_eq!(w.word(0xf0f8), 0x1234);
    w.sync(edge(&c, 6), &c).unwrap();
    assert_eq!(w.word(0xf0f8), 5);
    assert_eq!(w.word(0xf0fc), 0x1234);
    assert_eq!(
        w.read_word(0xf0f8, &c).unwrap(),
        0x1234,
        "same capture boundary reads old register"
    );
    assert_eq!(w.peek(0xf0f3) & 0xf, 1);
    w.sync(edge(&c, 7), &c).unwrap();
    assert_eq!(w.read_word(0xf0f8, &c).unwrap(), 5);
}
#[test]
fn stopped_counter_still_captures_and_asserts_interrupt() {
    let c = clocks();
    let mut w = capture(&c, false);
    w.write(0xf0f2, 1, edge(&c, 3), &c).unwrap();
    w.sync(edge(&c, 6), &c).unwrap();
    assert_eq!(w.word(0xf0f8), 123);
    assert!(w.interrupt());
    w.set_gate(false, edge(&c, 7), &c).unwrap();
    w.read(0xf0f3);
    w.write(0xf0f3, 0, edge(&c, 8), &c).unwrap();
    assert!(
        w.interrupt(),
        "cannot clear pending flag during module standby"
    );
}
#[test]
fn capture_write_collisions_keep_flags_and_cpu_data() {
    let c = clocks();
    for reg in [0xf0f8, 0xf0fc] {
        let mut w = capture(&c, true);
        w.write_word(reg, 0xabcd, edge(&c, 6), &c).unwrap();
        assert_eq!(w.word(reg), 0xabcd);
        assert_eq!(w.peek(0xf0f3) & 1, 1);
        assert_eq!(
            w.word(if reg == 0xf0f8 { 0xf0fc } else { 0xf0f8 }),
            if reg == 0xf0f8 { 0x1234 } else { 5 }
        );
    }
}
#[test]
fn external_clock_passes_through_synchronizer_and_same_counter_rules() {
    let c = clocks();
    let mut w = timer(&c);
    w.write(0xf0f1, 0x70, Time::ZERO, &c).unwrap();
    w.write(0xf0f0, 0x80, Time::ZERO, &c).unwrap();
    w.input_pins([None, None, None, None, Some(false)], Time::ZERO, &c)
        .unwrap();
    w.input_pins([None, None, None, None, Some(true)], edge(&c, 1), &c)
        .unwrap();
    w.sync(edge(&c, 3), &c).unwrap();
    assert_eq!(w.word(0xf0f6), 0);
    w.sync(edge(&c, 4), &c).unwrap();
    assert_eq!(w.word(0xf0f6), 1);
    w.input_pins([None, None, None, None, Some(false)], edge(&c, 5), &c)
        .unwrap();
    w.input_pins([None, None, None, None, Some(true)], edge(&c, 9), &c)
        .unwrap();
    w.sync(edge(&c, 12), &c).unwrap();
    assert_eq!(w.word(0xf0f6), 2);
}
#[test]
fn timer_w_watch_clock_runs_in_subsleep_but_not_watch_or_stabilization() {
    let mut m = Mcu::new(&vec![0; 49152], Default::default()).unwrap();
    m.write8(0xf0f1, 0x40, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    m.write8(0xf0f0, 0x80, WriteOrigin::MovByte, Time::ZERO, &mut ())
        .unwrap();
    m.control.gate2 |= 0x40;
    for (mode, stabilizing, runs) in [
        (Mode::Active, None, true),
        (Mode::Sleep, None, true),
        (Mode::Subactive, None, true),
        (Mode::Subsleep, None, true),
        (Mode::Watch, None, false),
        (Mode::Standby, None, false),
        (Mode::Active, Some(Mode::Watch), false),
    ] {
        m.control.mode = mode;
        m.control.stabilizing_from = stabilizing;
        m.apply_gates(Time::ZERO, &mut ()).unwrap();
        assert_eq!(
            m.timer_w.deadline(&m.clocks).unwrap().is_some(),
            runs,
            "{mode:?} {stabilizing:?}"
        );
    }
}
#[test]
fn buffer_and_capture_advancement_is_partition_invariant() {
    let c = clocks();
    let a = capture(&c, true);
    let mut b = a.clone();
    let mut a = a;
    a.sync(edge(&c, 10000), &c).unwrap();
    for i in 4..=10000 {
        b.sync(edge(&c, i), &c).unwrap();
    }
    assert_eq!(a, b);
}

#[test]
fn guest_gpio_edge_captures_stopped_timer_and_vectors_through_35() {
    use hs_core::{Images, Machine};
    let mut rom = vec![0u8; 49152];
    rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
    rom[70..72].copy_from_slice(&0x200u16.to_be_bytes());
    let mut p = vec![0x7a, 0x07, 0, 0, 0xff, 0x70];
    let byte = |p: &mut Vec<u8>, a: u16, v: u8| {
        p.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    };
    byte(&mut p, 0xfffb, 0x44); // Timer W and watchdog module clocks.
    byte(&mut p, 0xffe4, 7); // P10, P11, P12 outputs.
    byte(&mut p, 0xffd4, 4); // P10 low; EEPROM remains deselected.
    p.extend([0x79, 0x00, 0x12, 0x34, 0x6b, 0x80, 0xf0, 0xf6]); // TCNT=0x1234, CTS=0.
    byte(&mut p, 0xf0f4, 0x8c); // A capture rising.
    byte(&mut p, 0xf0f2, 1); // A IRQ enabled.
    byte(&mut p, 0xffd4, 5); // GPIO drives FTIOA high through the board route.
    p.extend([0x06, 0x7f, 0x01, 0x80, 0x40, 0xfc]);
    rom[0x100..0x100 + p.len()].copy_from_slice(&p);
    let h = [
        0x6b, 0x00, 0xf0, 0xf8, 0x6b, 0x80, 0xf7, 0x80, 0x6a, 0x08, 0xf0, 0xf3, 0xf8, 0, 0x6a,
        0x88, 0xf0, 0xf3, 0x56, 0x70,
    ];
    rom[0x200..0x200 + h.len()].copy_from_slice(&h);
    let mut a = Machine::new(Images {
        firmware: &rom,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap();
    let mut b = a.clone();
    let (mut x, mut y) = (Vec::new(), Vec::new());
    a.run_until(Time::from_micros(500), &[], &mut x).unwrap();
    for us in 1..=500 {
        b.run_until(Time::from_micros(us), &[], &mut y).unwrap();
    }
    assert_eq!(
        [a.peek(0xf780).unwrap(), a.peek(0xf781).unwrap()],
        [0x12, 0x34]
    );
    assert_eq!(a.interrupt_entries(), 1);
    assert_eq!(a, b);
    assert_eq!(x, y);
}

#[test]
fn expired_capture_visibility_does_not_survive_a_clock_epoch_change() {
    let mut c = clocks();
    let mut w = capture(&c, true);
    let now = edge(&c, 12);
    w.sync(now, &c).unwrap();
    c.select_system(now, hs_core::diagnostic::mcu::clocks::Source::Watch, 1)
        .unwrap();
    w.sync(now, &c).unwrap();
    assert_eq!(w.read_word(0xf0f8, &c).unwrap(), 5);
}

#[test]
fn switching_a_low_internal_clock_to_a_high_one_increments_tcnt() {
    let c = clocks();
    let mut w = timer(&c);
    w.write(0xf0f1, 0x20, Time::ZERO, &c).unwrap(); // phi/4
    w.write(0xf0f0, 0x80, Time::ZERO, &c).unwrap();
    let now = edge(&c, 2);
    w.sync(now, &c).unwrap();
    assert_eq!(w.read_word(0xf0f6, &c).unwrap(), 0);
    w.write(0xf0f1, 0x10, now, &c).unwrap(); // phi/4 low -> phi/2 high
    assert_eq!(w.read_word(0xf0f6, &c).unwrap(), 1);
    w.sync(edge(&c, 4), &c).unwrap();
    assert_eq!(w.read_word(0xf0f6, &c).unwrap(), 2);
}
