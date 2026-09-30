//! Persistence behavior through the public API. These original guests cover
//! unfinished bus work; the suite supplies independent hardware expectations.
#[path = "support/state.rs"]
mod state;
use hs_core::{Conditions, Frequencies, Images, Machine, Snapshot, Time};

fn machine(words: &[u16]) -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&[1, 0]);
    rom[16..18].copy_from_slice(&[2, 0]);
    rom[0x200..0x202].copy_from_slice(&[0x56, 0x70]); // RTE
    for (i, word) in words.iter().enumerate() {
        rom[0x100 + i * 2..0x102 + i * 2].copy_from_slice(&word.to_be_bytes());
    }
    rom[0x400..0x404].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[0xff; 65536],
            eeprom_status: 0,
            sensor_nonvolatile: None,
        },
        Conditions {
            clocks: Frequencies {
                main_hz: 1_000_000,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn each_quarter_cycle_replays_split_lanes_copy_exception_and_latched_arithmetic() {
    let mut m = machine(&[
        0x7907, 0xff70, // SP
        0x7900, 0x1234, 0x6b80, 0xf066, // split SFR word store
        0x7a00, 0x1122, 0x3344, 0x0100, 0x6b80, 0xf780, // long store
        0x0100, 0x6b01, 0xf780, // long load
        0x7904, 4, 0x7905, 0x400, 0x7906, 0xf790, 0x7bd4, 0x598f, // EEPMOV.W
        0x7f80, 0x7000, // BSET #0 at absolute-8 memory
        0xf809, 0x7901, 3, 0x5081, // MULXU.B R0L,R1 (latched result interval)
        0x5700, // TRAPA, exception stack and RTE
        0x6b81, 0xf7a0, 0x0180, 0x40fc, // publish result, SLEEP
    ]);
    let end = Time::from_micros(400);
    for quarter in 0..1000u128 {
        let at = Time::from_raw((quarter << 64) / 4_000_000);
        m.run_until(at, &[], &mut ()).unwrap();
        let snapshot = m.snapshot();
        let mut resumed = state::restore_file(&snapshot);
        let mut direct = m.clone();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        direct.run_until(end, &[], &mut a).unwrap();
        resumed.run_until(end, &[], &mut b).unwrap();
        assert_eq!(a, b, "effects after capture at {at:?}");
        assert_eq!(&direct.ram()[0x10..0x14], &[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(&direct.ram()[0x20..0x22], &[0, 27]);
        state::assert_same_state(&direct, &resumed);
        assert_eq!(snapshot, m.snapshot(), "capture must not advance hardware");
    }
}
#[test]
fn free_running_rtc_overflow_enable_and_flag_survive_files() {
    let mut m = machine(&[
        0xf800, 0x6a88, 0xf06f, // RTC free counter, phi/8
        0xf880, 0x6a88, 0xf06d, // free-counter overflow interrupt
        0x6a88, 0xf06c, // run
        0x40fe,
    ]);
    m.run_until(Time::from_micros(250), &[], &mut ()).unwrap();
    let mut loaded = state::restore_file(&m.snapshot());
    for machine in [&mut m, &mut loaded] {
        machine
            .run_until(Time::from_micros(3000), &[], &mut ())
            .unwrap();
        assert_ne!(machine.peek(0xf067).unwrap() & 0x80, 0);
    }
    state::assert_same_state(&m, &loaded);
    let loaded = state::restore_file(&m.snapshot());
    assert_eq!(loaded.peek(0xf067).unwrap(), m.peek(0xf067).unwrap());
}
#[test]
fn invalid_files_and_wrong_firmware_never_replace_the_session() {
    let mut a = machine(&[0x40fe]);
    a.run_until(Time::from_micros(50), &[], &mut ()).unwrap();
    let before = a.snapshot();
    let bytes = before.encode().unwrap();
    for n in [0, 7, 12, 43, bytes.len() - 1] {
        assert!(Snapshot::decode(&bytes[..n]).is_err());
    }
    for i in [0, 8, 12, 44, bytes.len() - 1] {
        let mut bad = bytes.clone();
        bad[i] ^= 1;
        assert!(Snapshot::decode(&bad).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Snapshot::decode(&trailing).is_err());
    let other = machine(&[0x0000, 0x40fc]).snapshot();
    assert!(a.restore(&other).is_err());
    assert_eq!(before, a.snapshot());
    a.restore(&Snapshot::decode(&bytes).unwrap()).unwrap();
    assert_eq!(a.retired(), 0);
    assert_eq!(a.snapshot().encode().unwrap(), bytes);
}
#[test]
fn faulted_instruction_capture_stays_stopped() {
    let mut m = machine(&[0x0100, 0xffff]);
    assert!(m.run_until(Time::from_micros(100), &[], &mut ()).is_err());
    let mut loaded = state::restore_file(&m.snapshot());
    let time = loaded.now();
    let mut events = vec![];
    assert!(loaded.power_on(&mut events).is_err());
    assert!(loaded.power_off(&mut events).is_err());
    assert!(events.is_empty());
    assert!(loaded
        .run_until(Time::from_micros(200), &[], &mut ())
        .is_err());
    assert_eq!(loaded.now(), time);
    state::assert_same_state(&m, &loaded);
}
#[test]
#[cfg_attr(
    target_family = "wasm",
    ignore = "requires native thread stack control"
)]
fn decode_uses_fixed_heap_storage_on_a_small_stack() {
    let bytes = machine(&[0x40fe]).snapshot().encode().unwrap();
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let loaded = Snapshot::decode(&bytes).unwrap();
            assert_eq!(loaded.encode().unwrap(), bytes);
        })
        .unwrap()
        .join()
        .unwrap();
}
