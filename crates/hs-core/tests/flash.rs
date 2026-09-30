//! Flash register, pulse and persistent cell observations.
use hs_core::{
    diagnostic::mcu::{control::Mode, flash::Flash},
    Event, NvDomain, Time,
};

fn t(us: u64) -> Time {
    Time::from_micros(us)
}
fn reg(f: &mut Flash, a: u16, v: u8, us: u64) {
    f.write_register(a, v, t(us), &mut ()).unwrap();
}
fn control(f: &mut Flash, v: u8, us: u64) {
    reg(f, 0xf020, v, us);
}
fn write(f: &mut Flash, a: u16, v: u8, us: u64) {
    f.write8(a, v, t(us), &mut ()).unwrap();
}
fn setup(f: &mut Flash, us: u64) {
    reg(f, 0xf02b, 0x80, us);
    control(f, 0x40, us);
}
fn pulse(f: &mut Flash, start: u64, length: u64) {
    control(f, 0x50, start);
    control(f, 0x51, start + 60);
    control(f, 0x50, start + 60 + length);
    control(f, 0x40, start + 65 + length);
}

#[test]
fn register_gates_masks_and_erase_selection_do_not_mutate_the_array() {
    let mut f = Flash::new(&[0xa5; 49152]).unwrap();
    reg(&mut f, 0xf020, 0x7f, 0);
    assert_eq!(f.register(0xf020), 0);
    setup(&mut f, 0);
    reg(&mut f, 0xf023, 0x20, 2);
    assert_eq!(f.register(0xf023), 0x20);
    reg(&mut f, 0xf023, 0x30, 3);
    assert_eq!(f.register(0xf023), 0);
    reg(&mut f, 0xf023, 0x80, 4);
    reg(&mut f, 0xf022, 0xff, 4);
    assert_eq!(f.register(0xf023), 0x80);
    assert_eq!(f.register(0xf022), 0x80);
    reg(&mut f, 0xf02b, 0, 5);
    reg(&mut f, 0xf020, 0, 6);
    assert_eq!(f.register(0xf020), 0);
    reg(&mut f, 0xf02b, 0xff, 7);
    assert_eq!(f.register(0xf020), 0x40);
    assert_eq!(f.register(0xf02b), 0x80);
    control(&mut f, 0xff, 8); // Conflicting pulse/verify requests retain their bits.
    assert_eq!(f.register(0xf020), 0x7f);
    assert_eq!(f.read8(0x1000, t(10000), &mut ()).unwrap(), 0xff);
    assert_eq!(f.register(0xf021), 0);
    control(&mut f, 0x3f, 10001);
    assert_eq!(f.register(0xf020), 0);
    assert_eq!(f.register(0xf023), 0);
    assert_eq!(f.read8(0x1000, t(10102), &mut ()).unwrap(), 0xa5);
    assert_eq!(&*f.image(t(20000)), &[0xa5; 49152]);
}

#[test]
fn program_latch_uses_the_last_page_and_retains_unloaded_slots() {
    let mut f = Flash::new(&[0xff; 49152]).unwrap();
    setup(&mut f, 0);
    write(&mut f, 0x1000, 0, 2);
    write(&mut f, 0x1081, 0x5a, 3);
    pulse(&mut f, 4, 7001);
    let image = f.image(t(8000));
    assert_eq!(image[0x1000], 0xff);
    assert_eq!(&image[0x1080..0x1083], &[0, 0x5a, 0xff]);
    // A one in the latch cannot undo a previously programmed zero.
    write(&mut f, 0x1080, 0xff, 8000);
    write(&mut f, 0x1081, 0xff, 8001);
    pulse(&mut f, 8010, 7001);
    assert_eq!(f.image(t(16000)), image);
}

#[test]
fn verify_senses_a_quadword_after_settling_and_keeps_the_previous_latch() {
    let mut bytes = [0xff; 49152];
    bytes[0x9000..0x9008].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]);
    let mut f = Flash::new(&bytes).unwrap();
    setup(&mut f, 0);
    control(&mut f, 0x44, 2);
    write(&mut f, 0x9000, 0xff, 3);
    assert_eq!(f.read16(0x9000, t(5), &mut ()).unwrap(), 0xffff);
    assert_eq!(f.read16(0x9000, t(7), &mut ()).unwrap(), 0x1234);
    assert_eq!(f.read16(0x9002, t(7), &mut ()).unwrap(), 0x5678);
    write(&mut f, 0x9004, 0x00, 10);
    assert_eq!(f.read16(0x9004, t(11), &mut ()).unwrap(), 0x1234);
    assert_eq!(f.read16(0x9004, t(13), &mut ()).unwrap(), 0x9abc);
    assert_eq!(f.read16(0x8012, t(14), &mut ()).unwrap(), 0xdef0); // Low address lanes, no retarget.
    assert_eq!(&*f.image(t(20)), &bytes);
}

#[test]
fn cumulative_exposure_survives_reset_and_verify_is_stricter_than_normal_sense() {
    let mut f = Flash::new(&[0xff; 49152]).unwrap();
    setup(&mut f, 0);
    write(&mut f, 0x9000, 0, 2);
    pulse(&mut f, 3, 2000);
    let first = f.image(t(2200))[0x9000];
    f.reset(t(2300), &mut ());
    setup(&mut f, 2400);
    write(&mut f, 0x9000, 0, 2402);
    pulse(&mut f, 2403, 2000);
    assert_eq!(f.read8(0x9000, t(4500), &mut ()).unwrap(), 0);
    assert_ne!(first, 0, "a single short pulse must not complete the page");
    control(&mut f, 0x44, 4501);
    write(&mut f, 0x9000, 0xff, 4506);
    assert_ne!(f.read8(0x9000, t(4509), &mut ()).unwrap(), 0);
    control(&mut f, 0x40, 4510);
    pulse(&mut f, 4520, 3001);
    control(&mut f, 0x44, 7600);
    write(&mut f, 0x9000, 0xff, 7605);
    assert_eq!(f.read8(0x9000, t(7608), &mut ()).unwrap(), 0);
}

#[test]
fn any_array_read_aborts_a_pulse_but_inspection_and_hiding_controls_do_not() {
    let mut f = Flash::new(&[0xff; 49152]).unwrap();
    setup(&mut f, 0);
    write(&mut f, 0x9000, 0, 2);
    control(&mut f, 0x50, 3);
    control(&mut f, 0x51, 63);
    reg(&mut f, 0xf02b, 0, 100);
    let snapshot = f.clone();
    let projected = f.image(t(4063));
    assert_eq!(projected[0x9000], 0);
    assert_eq!(f, snapshot, "export is a projection");
    assert_eq!(f.read16(0x100, t(2063), &mut ()).unwrap(), 0xffff); // A different block.
    reg(&mut f, 0xf02b, 0x80, 2064);
    assert_eq!(f.register(0xf021), 0x80);
    assert_eq!(f.register(0xf020), 0x51);
    let partial = f.image(t(2100));
    reg(&mut f, 0xf021, 0, 2101);
    control(&mut f, 0x50, 2102);
    control(&mut f, 0x51, 2200);
    assert_eq!(f.image(t(10000)), partial);
    f.reset(t(10001), &mut ());
    setup(&mut f, 10002);
    assert_eq!(f.register(0xf021), 0);
    assert_eq!(f.image(t(10003)), partial);
}

#[test]
fn erase_targets_the_38606_blocks_and_partial_charge_survives_power_loss() {
    let mut f = Flash::new(&[0; 49152]).unwrap();
    setup(&mut f, 0);
    reg(&mut f, 0xf023, 0x10, 2);
    control(&mut f, 0x60, 3);
    control(&mut f, 0x62, 104);
    f.power_off(t(70104), &mut ());
    let partial = f.image(t(200000));
    assert_eq!(partial[0xfff], 0);
    assert_eq!(partial[0x8000], 0);
    assert!(partial[0x1000..0x8000].iter().any(|b| *b != 0));
    assert!(partial[0x1000..0x8000].iter().any(|b| *b != 0xff));
    f.environment(Mode::Active, true, true, t(200001), &mut ())
        .unwrap();
    setup(&mut f, 200002);
    reg(&mut f, 0xf023, 0x10, 200004);
    control(&mut f, 0x60, 200005);
    control(&mut f, 0x62, 200106);
    control(&mut f, 0x60, 230107);
    let done = f.image(t(240000));
    assert_eq!(&done[0x1000..0x8000], &[0xff; 0x7000]);
    assert_eq!(done[0xfff], 0);
    assert_eq!(done[0x8000], 0);
}

#[test]
fn module_standby_initializes_controls_and_has_its_own_wake_settling() {
    let mut f = Flash::new(&[0xa5; 49152]).unwrap();
    setup(&mut f, 0);
    reg(&mut f, 0xf022, 0x80, 2);
    reg(&mut f, 0xf023, 0x20, 3);
    f.environment(Mode::Active, false, true, t(10), &mut ())
        .unwrap();
    assert_eq!(f.read8(0x100, t(15), &mut ()).unwrap(), 0xff);
    assert_eq!(f.register(0xf020), 0);
    assert_eq!(f.register(0xf023), 0);
    assert_eq!(f.register(0xf022), 0x80);
    assert_eq!(f.register(0xf02b), 0x80);
    f.environment(Mode::Active, true, true, t(20), &mut ())
        .unwrap();
    assert_eq!(f.read8(0x100, t(39), &mut ()).unwrap(), 0xff);
    assert_eq!(f.read8(0x100, t(41), &mut ()).unwrap(), 0xa5);
    f.environment(Mode::Subactive, true, false, t(50), &mut ())
        .unwrap();
    assert_eq!(f.read8(0x100, t(51), &mut ()).unwrap(), 0xa5);
}

fn programming() -> Flash {
    let mut f = Flash::new(&[0xff; 49152]).unwrap();
    setup(&mut f, 0);
    write(&mut f, 0x9000, 0, 2);
    control(&mut f, 0x50, 3);
    control(&mut f, 0x51, 63);
    f
}

#[test]
fn normal_pulse_reports_changed_cells_then_its_extent_without_polling() {
    let mut f = programming();
    let mut events = vec![];
    f.write_register(0xf020, 0x50, t(7064), &mut events)
        .unwrap();
    assert_eq!(
        events,
        [
            Event::NvByte {
                at: t(7064),
                domain: NvDomain::InternalFlash,
                address: 0x9000,
                value: 0
            },
            Event::NvCommit {
                at: t(7064),
                domain: NvDomain::InternalFlash,
                address: 0x9000,
                length: 128
            },
        ]
    );
    assert_eq!(f.image(t(8000))[0x9000], 0);
    // An identical closing write cannot close the same pulse twice.
    f.write_register(0xf020, 0x50, t(7065), &mut events)
        .unwrap();
    assert_eq!(events.len(), 2);
}

#[test]
fn interrupted_pulses_deliver_partial_cells_once_for_every_exit() {
    for cause in ["reset", "power", "module", "oscillator", "read", "bias"] {
        let mut f = programming();
        let before = f.clone();
        let image = f.image(t(4063));
        assert_eq!(before, f, "inspection must remain silent");
        assert_eq!(image[0x9000], 0);
        let mut events = vec![];
        match cause {
            "reset" => f.reset(t(4063), &mut events),
            "power" => f.power_off(t(4063), &mut events),
            "module" => f
                .environment(Mode::Active, false, true, t(4063), &mut events)
                .unwrap(),
            "oscillator" => f
                .environment(Mode::Active, true, false, t(4063), &mut events)
                .unwrap(),
            "read" => {
                f.read16(0x100, t(4063), &mut events).unwrap();
            }
            "bias" => f.write_register(0xf020, 0, t(4063), &mut events).unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(
            events,
            [
                Event::NvByte {
                    at: t(4063),
                    domain: NvDomain::InternalFlash,
                    address: 0x9000,
                    value: 0
                },
                Event::NvInterrupted {
                    at: t(4063),
                    domain: NvDomain::InternalFlash,
                    address: 0x9000,
                    length: 128
                },
            ],
            "{cause}"
        );
        f.reset(t(4100), &mut events);
        assert_eq!(events.len(), 2, "{cause} must not repeat its interruption");
        assert_eq!(f.image(t(5000)), image);
    }
}

#[test]
fn retargeted_erase_reports_each_range_and_preserves_all_intermediate_bytes() {
    let mut f = Flash::new(&[0; 49152]).unwrap();
    setup(&mut f, 0);
    reg(&mut f, 0xf023, 1, 2);
    control(&mut f, 0x60, 3);
    control(&mut f, 0x62, 104);
    let mut events = vec![];
    f.write_register(0xf023, 2, t(70104), &mut events).unwrap();
    assert_eq!(
        events.last(),
        Some(&Event::NvInterrupted {
            at: t(70104),
            domain: NvDomain::InternalFlash,
            address: 0,
            length: 0x400,
        })
    );
    let partial = f.image(t(70104));
    assert!(partial[..0x400].iter().any(|v| *v != 0));
    assert!(partial[..0x400].iter().any(|v| *v != 0xff));
    f.write_register(0xf020, 0x60, t(180105), &mut events)
        .unwrap();
    assert_eq!(
        events.last(),
        Some(&Event::NvCommit {
            at: t(180105),
            domain: NvDomain::InternalFlash,
            address: 0x400,
            length: 0x400,
        })
    );
    let mut persisted = [0; 49152];
    for event in events {
        if let Event::NvByte {
            domain: NvDomain::InternalFlash,
            address,
            value,
            ..
        } = event
        {
            persisted[usize::from(address)] = value;
        }
    }
    assert_eq!(&persisted[..0x400], &partial[..0x400]);
    assert_eq!(&persisted[0x400..0x800], &[0xff; 0x400]);
    assert_eq!(persisted, *f.image(t(181000)));
}
