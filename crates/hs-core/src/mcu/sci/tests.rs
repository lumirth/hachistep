use super::super::clocks::{Frequencies, Source};
use super::*;

fn clocks(hz: u64) -> Clocks {
    Clocks::new(
        Time::ZERO,
        Frequencies {
            main_hz: hz,
            watch_hz: 1_000,
            on_chip_hz: 1_000_000,
        },
    )
    .unwrap()
}
fn configured(c: &Clocks, smr: u8, semr: u8, ircr: u8, scr: u8) -> Sci {
    let mut s = Sci::default();
    s.set_power(true, true, false, false, Time::ZERO, c)
        .unwrap();
    for (a, v) in [
        (0xff91, 0xd0),
        (0xff98, smr),
        (0xff99, 0),
        (0xffa6, semr),
        (0xffa7, ircr),
        (0xff9a, scr),
    ] {
        s.write(a, v, Time::ZERO, c).unwrap();
    }
    s
}
fn run(s: &mut Sci, c: &Clocks, end: Time) {
    let mut count = 0;
    while let Some(at) = s.deadline(c).unwrap().filter(|at| *at <= end) {
        s.advance(at, c).unwrap();
        count += 1;
        assert!(count < 10000, "SCI failed to leave its appointment");
    }
    s.sync(end, c).unwrap();
}
fn write(s: &mut Sci, c: &Clocks, us: u64, a: u16, v: u8) {
    let at = Time::from_micros(us);
    run(s, c, at);
    s.write(a, v, at, c).unwrap();
}
fn tenth(us: u64) -> Time {
    Time::from_raw((u128::from(us) << 64) / 10_000_000)
}
fn input(s: &mut Sci, c: &Clocks, at: Time, high: bool) {
    run(s, c, at);
    s.input_pins(None, high, at, c).unwrap();
}
// The expectations below are literal register/timing-table consequences from
// REJ09B0152-0300 and A333B/E, not values computed by Format or Baud helpers.
#[test]
fn startup_mark_and_first_stop_status_are_separate_from_wire_completion() {
    let c = clocks(1_000_000);
    let mut s = configured(&c, 0, 0, 0, 0x20);
    s.write(0xff9b, 0xa5, Time::ZERO, &c).unwrap();
    run(&mut s, &c, Time::from_micros(319));
    assert_eq!(s.pins().transmit, Some(true));
    assert_eq!(s.ssr, 0);
    for (cell, high) in [
        false, true, false, true, false, false, true, false, true, true,
    ]
    .into_iter()
    .enumerate()
    {
        run(&mut s, &c, Time::from_micros(320 + cell as u64 * 32));
        assert_eq!(s.pins().transmit, Some(high));
        assert_eq!(s.ssr, if cell == 9 { 0x84 } else { 0x80 });
        assert_eq!(s.transmitted, 0);
    }
    run(&mut s, &c, Time::from_micros(640));
    assert_eq!(s.transmitted, 1);
    assert_eq!(s.deadline(&c).unwrap(), None);
}
#[test]
fn tdr_during_stop_cannot_replace_the_preloaded_tsr() {
    let c = clocks(1_000_000);
    for (smr, start, tail) in [(0, 320, 32), (8, 352, 64)] {
        let mut s = configured(&c, smr, 0, 0, 0x20);
        s.write(0xff9b, 0xa5, Time::ZERO, &c).unwrap();
        write(&mut s, &c, start + 280, 0xff9b, 0x3c);
        run(&mut s, &c, Time::from_micros(start + 288));
        assert_eq!(s.ssr, 0x80);
        write(&mut s, &c, start + 292, 0xff9b, 0xc3);
        assert_eq!(s.ssr, 0);
        for (cell, high) in [
            false, false, false, true, true, true, true, false, false, true,
        ]
        .into_iter()
        .enumerate()
        {
            run(
                &mut s,
                &c,
                Time::from_micros(start + 288 + tail + cell as u64 * 32),
            );
            assert_eq!(s.pins().transmit, Some(high));
        }
        let third = start + 288 + tail + 288 + tail;
        run(&mut s, &c, Time::from_micros(third + 32));
        assert_eq!(s.pins().transmit, Some(true));
        assert_eq!(s.transmitted, 2);
    }
}
#[test]
fn corrected_five_bit_formats_and_reserved_decoder() {
    let c = clocks(1_000_000);
    for (smr, cells) in [
        (0x24, vec![false, true, false, true, false, true, true]),
        (
            0x64,
            vec![false, true, false, true, false, true, true, true],
        ),
        (
            0x74,
            vec![false, true, false, true, false, true, false, true],
        ),
        (0x04, vec![false, true, false, true, false, true, true]),
    ] {
        let mut s = configured(&c, smr, 0, 0, 0x20);
        s.write(0xff9b, 0xf5, Time::ZERO, &c).unwrap();
        let start = cells.len() as u64 * 32;
        for (i, high) in cells.into_iter().enumerate() {
            run(&mut s, &c, Time::from_micros(start + i as u64 * 32));
            assert_eq!(s.pins().transmit, Some(high), "SMR {smr:02x}, cell {i}");
        }
    }
}
#[test]
fn sampled_start_depends_on_basic_clock_phase_and_rejects_a_glitch() {
    let c = clocks(2_000_000);
    for (fall, validation) in [(4, 8), (6, 9)] {
        let mut s = configured(&c, 0, 0, 0, 0x10);
        input(&mut s, &c, tenth(fall), false);
        run(&mut s, &c, Time::from_micros(validation));
        assert_eq!(
            s.deadline(&c).unwrap(),
            Some(Time::from_micros(validation + 16))
        );
    }
    let mut s = configured(&c, 0, 0, 0, 0x10);
    input(&mut s, &c, tenth(6), false);
    input(&mut s, &c, tenth(89), true);
    run(&mut s, &c, Time::from_micros(9));
    assert_eq!(s.ssr, 0x84);
    assert_eq!(s.deadline(&c).unwrap(), None);
}
fn receive(
    s: &mut Sci,
    c: &Clocks,
    start: u64,
    data: u8,
    parity: Option<bool>,
    stop: bool,
) -> Time {
    // 2-MHz phi, BRR=0: sixteen microseconds per ordinary asynchronous bit.
    input(s, c, tenth(start), false);
    for i in 0..8 {
        input(s, c, tenth(start + (i + 1) * 160), data & (1 << i) != 0);
    }
    let last = if let Some(p) = parity {
        input(s, c, tenth(start + 1440), p);
        1600
    } else {
        1440
    };
    input(s, c, tenth(start + last), stop);
    let end = tenth(start + last + 90);
    run(s, c, end);
    end
}
#[test]
fn bad_bytes_transfer_without_rdrf_and_overrun_preserves_the_old_byte() {
    let c = clocks(2_000_000);
    let mut s = configured(&c, 0x20, 0, 0, 0x10);
    receive(&mut s, &c, 6, 0xa5, Some(true), true);
    assert_eq!((s.rdr, s.ssr), (0xa5, 0x8c));
    s.write(0xff9c, 0, Time::from_micros(170), &c).unwrap();
    assert_eq!(s.ssr, 0x8c); // no preceding read: error remains
    s.read(0xff9c);
    s.write(0xff9c, 0xf7, Time::from_micros(170), &c).unwrap();
    assert_eq!(s.ssr, 0x84);
    let mut s = configured(&c, 0, 0, 0, 0x10);
    receive(&mut s, &c, 6, 0xa5, None, true);
    receive(&mut s, &c, 2006, 0x3c, None, false);
    assert_eq!((s.rdr, s.ssr), (0xa5, 0xf4));
    assert_eq!(s.read(0xff9d), 0xa5);
    assert_eq!(s.ssr, 0xb4);
    let mut s = configured(&c, 0, 0, 0, 0x10);
    let end = receive(&mut s, &c, 6, 0xa5, None, false);
    assert_eq!((s.rdr, s.ssr), (0xa5, 0x94));
    s.read(0xff9c);
    s.write(0xff9c, 0xef, end, &c).unwrap();
    run(&mut s, &c, Time::from_micros(320));
    assert_eq!((s.rdr, s.ssr), (0, 0x94)); // held break starts again after clear
}
#[test]
fn external_synchronous_edges_shift_both_directions_and_pause_exactly() {
    let c = clocks(1_000_000);
    let mut s = configured(&c, 0x80, 0, 0, 0x32);
    s.write(0xff9b, 0xa5, Time::ZERO, &c).unwrap();
    assert_eq!(s.deadline(&c).unwrap(), None);
    for i in 0..8 {
        let fall = Time::from_micros(10 + i * 100);
        let rise = Time::from_micros(20 + i * 100);
        s.input_pins(Some(false), 0x3c & (1 << i) != 0, fall, &c)
            .unwrap();
        assert_eq!(s.pins().transmit, Some(0xa5 & (1 << i) != 0));
        assert_eq!(s.ssr, if i == 7 { 0x84 } else { 0x80 });
        s.input_pins(Some(true), 0x3c & (1 << i) != 0, rise, &c)
            .unwrap();
        let snapshot = s.clone();
        run(&mut s, &c, Time::from_micros(99 + i * 100));
        assert_eq!(
            (s.rdr, s.ssr, s.tx, s.rx),
            (snapshot.rdr, snapshot.ssr, snapshot.tx, snapshot.rx)
        );
    }
    assert_eq!((s.rdr, s.ssr, s.transmitted), (0x3c, 0xc4, 1));
    assert_eq!(s.pins().transmit, Some(true));
}
#[test]
fn receive_only_master_keeps_clocking_until_overrun() {
    let c = clocks(1_000_000);
    let mut s = configured(&c, 0x80, 0, 0, 0x10);
    for edge in 1..=32 {
        run(&mut s, &c, Time::from_micros(edge * 2));
        assert_eq!(
            s.pins().clock,
            Some(if edge % 2 == 0 {
                Drive::High
            } else {
                Drive::Low
            })
        );
        if edge == 16 {
            assert_eq!((s.rdr, s.ssr), (255, 0xc4));
        }
    }
    assert_eq!((s.rdr, s.ssr), (255, 0xe4));
    assert_eq!(s.deadline(&c).unwrap(), None);
}
#[test]
fn baud_writes_and_live_clock_changes_preserve_the_right_countdown() {
    let mut c = clocks(1_000_000);
    let mut s = configured(&c, 0, 0, 0, 1);
    assert_eq!(s.deadline(&c).unwrap(), Some(Time::from_micros(16)));
    write(&mut s, &c, 0, 0xff99, 3);
    assert_eq!(s.deadline(&c).unwrap(), Some(Time::from_micros(64)));
    write(&mut s, &c, 0, 0xff9a, 0x21);
    run(&mut s, &c, Time::from_micros(3));
    s.write(0xff98, 1, Time::from_micros(3), &c).unwrap();
    // One old BRC edge remains; it is now the next physical watch edge.
    assert_eq!(s.deadline(&c).unwrap(), Some(Time::from_micros(61_000)));
    let pause = Time::from_micros(500);
    run(&mut s, &c, pause);
    s.set_power(true, false, true, false, pause, &c).unwrap();
    c.select_system(pause, Source::Watch, 1).unwrap();
    assert_eq!(s.deadline(&c).unwrap(), Some(Time::from_micros(61_000)));
    // CKS00 is main phi, even while the CPU's reference has become phiW.
    s.write(0xff98, 0, pause, &c).unwrap();
    assert_eq!(s.deadline(&c).unwrap(), None);
}
#[test]
fn infrared_pulse_is_centered_and_fixed_width_uses_phi_not_brr() {
    let c = clocks(1_000_000);
    for (selector, width) in [(0, 6), (1, 2), (2, 4), (3, 8), (4, 16)] {
        let mut s = configured(&c, 0, 0, 0x80 | (selector << 4), 0x20);
        s.write(0xff9b, 0xff, Time::ZERO, &c).unwrap();
        run(&mut s, &c, Time::from_micros(332));
        assert_eq!(s.pins().transmit, Some(false));
        run(&mut s, &c, Time::from_micros(333));
        assert_eq!(s.pins().transmit, Some(true));
        let saved = s.clone();
        run(&mut s, &c, Time::from_micros(333 + width));
        assert_eq!(s.pins().transmit, Some(false));
        let mut restored = saved;
        run(&mut restored, &c, Time::from_micros(333 + width));
        assert_eq!(s, restored);
    }
}
#[test]
fn power_retention_live_inversion_and_sync_to_gpio_glitch() {
    let c = clocks(1_000_000);
    let mut s = configured(&c, 0, 0, 0, 0);
    assert_eq!(s.pins().transmit, Some(true));
    s.write(0xff91, 0xd2, Time::ZERO, &c).unwrap();
    assert_eq!(s.pins().transmit, Some(false));
    s.set_power(true, false, false, true, Time::ZERO, &c)
        .unwrap();
    assert_eq!((s.spcr, s.brr, s.ssr), (0xd2, 255, 0x84));
    s.set_power(false, false, false, false, Time::ZERO, &c)
        .unwrap();
    assert_eq!(s.spcr, 0xc0);
    let mut s = configured(&c, 0x80, 0, 0, 0);
    s.write(0xff98, 0, Time::ZERO, &c).unwrap();
    assert_eq!(s.pins().clock, Some(Drive::Low));
    assert_eq!(s.deadline(&c).unwrap(), Some(tenth(5)));
    run(&mut s, &c, tenth(5));
    assert_eq!(s.pins().clock, None);
    let mut s = configured(&c, 0x80, 0, 0, 0);
    s.write(0xff9a, 2, Time::ZERO, &c).unwrap();
    s.write(0xff98, 0, Time::ZERO, &c).unwrap();
    s.write(0xff9a, 0, Time::ZERO, &c).unwrap();
    assert_eq!(s.pins().clock, None);
    assert_eq!(s.deadline(&c).unwrap(), None);
}

#[test]
fn external_async_clock_samples_and_can_pause_between_any_two_edges() {
    let c = clocks(1_000_000);
    for cke in [2, 3] {
        let mut s = configured(&c, 0, 0, 0, 0x10 | cke);
        s.input_pins(Some(true), false, Time::from_micros(100), &c)
            .unwrap();
        let mut paused = s.clone();
        for edge in 1..=304 {
            let cell = edge / 32;
            let high = if cell == 0 {
                false
            } else if cell <= 8 {
                0xa5 & (1 << (cell - 1)) != 0
            } else {
                true
            };
            s.input_pins(Some(edge % 2 == 0), high, Time::from_micros(100 + edge), &c)
                .unwrap();
            paused
                .input_pins(
                    Some(edge % 2 == 0),
                    high,
                    Time::from_micros(100 + edge + if edge >= 17 { 1000 } else { 0 }),
                    &c,
                )
                .unwrap();
            assert_eq!(s.deadline(&c).unwrap(), None);
            assert_eq!((s.rdr, s.ssr, s.rx), (paused.rdr, paused.ssr, paused.rx));
        }
        assert_eq!((s.rdr, s.ssr), (0xa5, 0xc4));
    }
}

#[test]
fn infrared_receiver_keeps_short_pulses_and_input_inversion_precedes_decoding() {
    let c = clocks(2_000_000);
    let mut s = configured(&c, 0, 0, 0, 0);
    s.write(0xff91, 0xd1, Time::ZERO, &c).unwrap();
    s.write(0xffa7, 0x80, Time::ZERO, &c).unwrap();
    s.write(0xff9a, 0x10, Time::ZERO, &c).unwrap();
    for cell in [0, 2, 4, 5, 7] {
        // One tenth of a microsecond is below the documented normal pulse
        // width. The target explicitly also recognizes shorter pulses.
        input(&mut s, &c, tenth(6 + 160 * cell), false);
        input(&mut s, &c, tenth(7 + 160 * cell), true);
    }
    run(&mut s, &c, Time::from_micros(153));
    assert_eq!((s.rdr, s.ssr), (0xa5, 0xc4));
}

#[test]
fn fixed_ir_pulse_holds_when_phi_stops_even_while_watch_baud_runs() {
    let c = clocks(1_000_000);
    let mut s = configured(&c, 1, 0, 0xa0, 0x20);
    s.write(0xff9b, 255, Time::ZERO, &c).unwrap();
    run(&mut s, &c, Time::from_micros(333_001));
    assert_eq!(s.pins().transmit, Some(true));
    s.set_power(true, false, true, false, Time::from_micros(333_001), &c)
        .unwrap();
    run(&mut s, &c, Time::from_micros(334_000));
    assert_eq!(s.pins().transmit, Some(true));
    s.set_power(true, true, false, false, Time::from_micros(334_000), &c)
        .unwrap();
    run(&mut s, &c, Time::from_micros(334_002));
    assert_eq!(s.pins().transmit, Some(true));
    run(&mut s, &c, Time::from_micros(334_003));
    assert_eq!(s.pins().transmit, Some(false));
}

#[test]
fn receive_format_is_latched_but_live_brr_reloads_after_the_old_count() {
    let c = clocks(2_000_000);
    let mut s = configured(&c, 0, 0, 0, 0x10);
    input(&mut s, &c, tenth(6), false);
    write(&mut s, &c, 10, 0xff98, 0x64);
    write(&mut s, &c, 10, 0xffa6, 8);
    for i in 0..8 {
        input(&mut s, &c, tenth(6 + (i + 1) * 160), 0xa5 & (1 << i) != 0);
    }
    input(&mut s, &c, tenth(1446), true);
    run(&mut s, &c, Time::from_micros(153));
    assert_eq!((s.rdr, s.ssr), (0xa5, 0xc4));
    let mut s = configured(&c, 0, 0, 0, 0);
    write(&mut s, &c, 0, 0xff99, 3);
    write(&mut s, &c, 0, 0xff9a, 0x20);
    write(&mut s, &c, 1, 0xff99, 0); // 2 phi edges remain of the original four.
    assert_eq!(
        s.deadline(&c).unwrap(),
        Some(Time::from_raw((323u128 << 64) / 2_000_000))
    );
}

#[test]
fn sync_receive_error_blocks_a_preloaded_next_transmit_byte_until_cleared() {
    let c = clocks(1_000_000);
    let mut s = configured(&c, 0x80, 0, 0, 0x10);
    run(&mut s, &c, Time::from_micros(32));
    assert_eq!(s.ssr, 0xc4);
    write(&mut s, &c, 32, 0xff9a, 0x30);
    write(&mut s, &c, 32, 0xff9b, 0xa5);
    write(&mut s, &c, 40, 0xff9b, 0x3c);
    run(&mut s, &c, Time::from_micros(64));
    assert_eq!((s.transmitted, s.ssr), (1, 0xe0));
    assert_eq!(s.deadline(&c).unwrap(), None);
    s.read(0xff9d);
    s.read(0xff9c);
    s.write(0xff9c, 0xdf, Time::from_micros(64), &c).unwrap();
    run(&mut s, &c, Time::from_micros(66));
    assert_eq!(s.pins().transmit, Some(false));
    assert_eq!(s.ssr, 0x80);
}
