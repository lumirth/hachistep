//! Timing-mechanism tests, not claims about unmeasured oscillator parameters.
use hs_core::{
    mcu::{
        adc::Adc,
        clocks::{ClockWait, Clocks, Frequencies, Tap},
        gpio::Gpio,
        ssu::Ssu,
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
fn plus(t: Time) -> Time {
    Time::from_raw(t.raw() + 1)
}

#[test]
fn an_outstanding_obligation_follows_the_new_clock_not_the_old_timestamp() {
    let mut c = clocks();
    let w = ClockWait::after(Time::ZERO, 10, Tap::system(1), &c).unwrap();
    let switch = c.edge(4, Tap::system(1)).unwrap();
    c.set_system(switch, 500_000, 1).unwrap();
    let due = w.deadline(&c).unwrap().unwrap();
    assert_eq!(due, c.edge(10, Tap::system(1)).unwrap());
    assert_eq!(c.ticks(Time::from_raw(due.raw() - 1), Tap::system(1)), 9);
    assert!(due.raw() > Time::from_micros(15).raw());
    assert!(due.raw() <= Time::from_micros(16).raw());
}

#[test]
fn paused_obligations_rejoin_the_divider_instead_of_replaying_wall_time() {
    let c = clocks();
    let mut w = ClockWait::after(Time::ZERO, 3, Tap::system(4), &c).unwrap();
    let pause = c.edge(5, Tap::system(1)).unwrap(); // One divider edge consumed.
    w.pause(pause, &c).unwrap();
    assert_eq!(w.deadline(&c).unwrap(), None);
    w.resume(c.edge(22, Tap::system(1)).unwrap(), &c).unwrap();
    assert_eq!(
        w.deadline(&c).unwrap(),
        Some(c.edge(28, Tap::system(1)).unwrap())
    );
}

#[test]
fn clock_wait_boundary_and_error_cases() {
    let c = clocks();
    assert!(ClockWait::after(Time::ZERO, 1, Tap::system(0), &c).is_err());
    let now = c.edge(13, Tap::system(1)).unwrap();
    let mut ready = ClockWait::after(now, 0, Tap::system(1), &c).unwrap();
    ready.pause(now, &c).unwrap();
    ready.resume(plus(now), &c).unwrap();
    assert_eq!(ready.deadline(&c).unwrap(), Some(plus(now)));
    let mut expired = ClockWait::after(Time::ZERO, 1, Tap::system(1), &c).unwrap();
    assert!(expired.pause(now, &c).is_err());
}

#[test]
fn adc_sample_and_result_obligations_survive_a_rate_change() {
    let mut c = clocks();
    let mut a = Adc::default();
    a.set_gate(true, Time::ZERO, &c).unwrap();
    a.write(0xffbe, 0x27, Time::ZERO, &c).unwrap(); // 31 system clocks.
    a.write(0xffbf, 0x80, Time::ZERO, &c).unwrap();
    let sample_time = a.deadline(&c).unwrap().unwrap();
    assert_eq!(sample_time, c.edge(4, Tap::system(1)).unwrap());
    assert!(!a.advance(sample_time, 411, &c).unwrap());
    let switch = c.edge(10, Tap::system(1)).unwrap();
    c.set_system(switch, 250_000, 1).unwrap();
    let finish = a.deadline(&c).unwrap().unwrap();
    assert_eq!(finish, c.edge(31, Tap::system(1)).unwrap());
    assert!(a.advance(finish, 999, &c).unwrap());
    assert_eq!(a.result(), 411 << 6);
}

#[test]
fn adc_gate_keeps_both_sample_and_finish_progress() {
    let c = clocks();
    let mut a = Adc::default();
    a.set_gate(true, Time::ZERO, &c).unwrap();
    a.write(0xffbe, 0x27, Time::ZERO, &c).unwrap();
    a.write(0xffbf, 0x80, Time::ZERO, &c).unwrap();
    a.set_gate(false, c.edge(2, Tap::system(1)).unwrap(), &c)
        .unwrap();
    assert_eq!(a.deadline(&c).unwrap(), None);
    a.set_gate(true, c.edge(100, Tap::system(1)).unwrap(), &c)
        .unwrap();
    assert_eq!(
        a.deadline(&c).unwrap(),
        Some(c.edge(102, Tap::system(1)).unwrap())
    );
    assert!(!a
        .advance(a.deadline(&c).unwrap().unwrap(), 100, &c)
        .unwrap());
    assert_eq!(
        a.deadline(&c).unwrap(),
        Some(c.edge(129, Tap::system(1)).unwrap())
    );
}

#[test]
fn ssu_partial_shifter_and_holding_register_survive_gating_and_clock_switch() {
    let mut c = clocks();
    let mut s = Ssu::default();
    s.set_gate(true, Time::ZERO, &c).unwrap();
    for (a, v) in [
        (0xf0e0, 0x8c),
        (0xf0e1, 0x40),
        (0xf0e2, 0x86),
        (0xf0e3, 0xc0),
        (0xf0eb, 0xa6),
    ] {
        s.write(a, v, Time::ZERO, &c).unwrap();
    }
    // Load plus three real edges, then gate with a partial byte still in flight.
    for _ in 0..4 {
        let t = s.deadline(&c).unwrap().unwrap();
        if let Some(e) = s.advance(t, &c).unwrap() {
            if e.sample {
                s.sample(e.mosi);
            }
            s.finish_edge(t, &c).unwrap();
        }
    }
    let pause = c.edge(7, Tap::system(1)).unwrap();
    s.set_gate(false, pause, &c).unwrap();
    assert_eq!(s.deadline(&c).unwrap(), None);
    let resume = c.edge(101, Tap::system(1)).unwrap();
    c.set_system(resume, 2_000_000, 1).unwrap();
    s.set_gate(true, resume, &c).unwrap();
    assert_eq!(
        s.deadline(&c).unwrap(),
        Some(c.edge(102, Tap::system(1)).unwrap())
    );
    while let Some(t) = s.deadline(&c).unwrap() {
        if let Some(e) = s.advance(t, &c).unwrap() {
            if e.sample {
                s.sample(e.mosi);
            }
            s.finish_edge(t, &c).unwrap();
        }
    }
    assert_eq!(s.read(0xf0e9), 0xa6);
    assert_eq!(s.transmitted, 1);
    assert_eq!(s.received, 1);
}

#[test]
fn ssu_switches_prescaler_with_a_byte_already_in_flight() {
    let c = clocks();
    let mut s = Ssu::default();
    s.set_gate(true, Time::ZERO, &c).unwrap();
    for (a, v) in [
        (0xf0e0, 0x8c),
        (0xf0e1, 0x40),
        (0xf0e2, 0x87),
        (0xf0e3, 0xc0),
        (0xf0eb, 0xa6),
    ] {
        s.write(a, v, Time::ZERO, &c).unwrap();
    }
    // Load, then six watch-clock half edges: a partial byte, not a restart.
    for _ in 0..7 {
        let t = s.deadline(&c).unwrap().unwrap();
        if let Some(e) = s.advance(t, &c).unwrap() {
            if e.sample {
                s.sample(e.mosi);
            }
            s.finish_edge(t, &c).unwrap();
        }
    }
    let switch = Time::from_micros(190);
    s.write(0xf0e2, 0x86, switch, &c).unwrap();
    assert_eq!(
        s.deadline(&c).unwrap(),
        Some(c.after(switch, 1, Tap::system(2)).unwrap())
    );
    let mut remaining = 0;
    while let Some(t) = s.deadline(&c).unwrap() {
        let e = s.advance(t, &c).unwrap().unwrap();
        if e.sample {
            s.sample(e.mosi);
        }
        s.finish_edge(t, &c).unwrap();
        remaining += 1;
    }
    assert_eq!(remaining, 10);
    assert_eq!(s.read(0xf0e9), 0xa6);
    assert_eq!(s.transmitted, 1);
    assert_eq!(s.received, 1);
}

#[test]
fn every_pull_register_accepts_writes_and_affects_only_input_configured_pins() {
    for (reg, dir, port, mask) in [
        (0xffe0, 0xffe4, 0xffd4, 7),
        (0xffe1, 0xffe6, 0xffd6, 7),
        (0xf086, 0xffeb, 0xffdb, 0x1c),
        (0xf087, 0xffec, 0xffdc, 15),
    ] {
        for value in 0..=255u8 {
            let mut p = Gpio::default();
            p.write(reg, value).unwrap();
            assert_eq!(p.read(reg), value & mask);
            p.resolve(None, 0, 0, None);
            assert_eq!(p.read(port) & value & mask, value & mask);
            p.write(dir, mask).unwrap();
            p.write(port, 0).unwrap();
            p.resolve(None, 0, 0, None);
            assert_eq!(p.read(port) & mask, 0);
        }
    }
}
