//! Original UART fixture for REJ09B0152-0300 §6.3/Table 6.2.
//! The hidden boot-ROM instruction schedule is not asserted.
#[path = "support/state.rs"]
mod state;
use hs_core::{DigitalPin, Images, Input, Machine, Time, TimedInput};
fn at(us: u64) -> Time {
    Time::from_micros(us)
}
fn pin(us: u64, input: Input) -> TimedInput {
    TimedInput { at: at(us), input }
}
fn uart(rows: &mut Vec<TimedInput>, us: u64, byte: u8) {
    for bit in 0..10u64 {
        rows.push(pin(
            us + bit * 1250 / 3,
            Input::DigitalPin {
                pin: DigitalPin::P31,
                level: Some(bit == 9 || (bit != 0 && byte & (1 << (bit - 1)) != 0)),
            },
        ));
    }
}
fn start() -> Vec<TimedInput> {
    let mut rows = vec![
        pin(0, Input::ResetPin(false)),
        pin(0, Input::NmiPin(false)),
        pin(100, Input::ResetPin(true)),
    ];
    uart(&mut rows, 1000, 0);
    uart(&mut rows, 20_000, 0x55);
    rows
}
fn upload(begin: u64, code: &[u8]) -> Vec<TimedInput> {
    let mut rows = start();
    for (i, byte) in (code.len() as u16)
        .to_be_bytes()
        .iter()
        .chain(code)
        .enumerate()
    {
        uart(&mut rows, begin + i as u64 * 6000, *byte);
    }
    rows
}
fn machine(fill: u8) -> Machine {
    Machine::new(Images {
        firmware: &vec![fill; 49152],
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
    })
    .unwrap()
}
const CODE: &[u8] = &[0x7a, 7, 0, 0, 0xff, 0x70, 0x01, 0x80, 0x40, 0xfe];
#[test]
fn upload_enters_the_same_cpu_and_preserves_adjusted_baud() {
    let rows = upload(50_000, CODE);
    let mut m = machine(255);
    m.run_until(at(130_000), &rows, &mut ()).unwrap();
    assert!(m.sleeping());
    assert_eq!(m.registers().sp(), 0xff70);
    assert_eq!(m.peek(0xff99).unwrap(), 0x2f);
    assert_eq!(m.peek(0xff9a).unwrap() & 0x30, 0);
    assert_eq!(m.peek(0xffd6).unwrap() & 4, 4);
    assert_eq!(m.peek(0xffe6).unwrap() & 4, 4);
    assert_eq!(&m.ram()[0x400..0x400 + CODE.len()], CODE);
    let mut split = machine(255);
    let mut consumed = 0;
    for us in [
        100, 131, 1800, 5000, 8000, 21000, 30000, 55500, 62123, 65000, 91000, 119123, 130000,
    ] {
        consumed += split
            .run_until(at(us), &rows[consumed..], &mut ())
            .unwrap()
            .inputs_consumed;
        split = state::restore_file(&split.snapshot());
    }
    state::assert_same_state(&m, &split);
}
#[test]
fn nonblank_boot_erases_every_block_with_retained_partial_pulses() {
    let rows = upload(1_100_000, CODE);
    let mut m = machine(0);
    m.run_until(at(50_000), &rows, &mut ()).unwrap();
    let mut resumed = state::restore_file(&m.snapshot());
    let suffix = rows.partition_point(|r| r.at < m.now());
    for m in [&mut m, &mut resumed] {
        let resets = m.statistics().resets;
        m.run_until(at(1_180_000), &rows[suffix..], &mut ())
            .unwrap();
        assert!(m.sleeping(), "phase {} at {:?}", m.phase_name(), m.now());
        assert!(m.firmware().iter().all(|b| *b == 255));
        assert_eq!(
            m.statistics().resets,
            resets,
            "WDT guard must exceed erase pulses"
        );
    }
    state::assert_same_state(&m, &resumed);
    // Erasing all blocks materializes every cell's charge: exercise the maximum
    // physical native payload, not merely a tiny initial snapshot.
    let bytes = m.snapshot().encode().unwrap();
    assert!(bytes.len() > 3_000_000 && bytes.len() <= hs_core::Snapshot::MAX_ENCODED_SIZE);
    let loaded = state::restore_file(&m.snapshot());
    state::assert_same_state(&m, &loaded);
}
#[test]
fn invalid_lengths_wait_for_reset_and_never_overwrite_past_the_aperture() {
    for length in [0u16, 1025, 65535] {
        let mut rows = start();
        for (i, byte) in length.to_be_bytes().into_iter().enumerate() {
            uart(&mut rows, 50_000 + i as u64 * 6000, byte);
        }
        uart(&mut rows, 62_000, 0x7a);
        let mut m = machine(255);
        m.run_until(at(100_000), &rows, &mut ()).unwrap();
        assert_eq!(m.phase_name(), "boot-service");
        assert!(m.ram().iter().all(|v| *v == 0));
        let mut loaded = state::restore_file(&m.snapshot());
        loaded.run_until(at(1_000_000), &[], &mut ()).unwrap();
        assert_eq!(loaded.statistics().resets, 0);
    }
}

#[test]
fn reset_aborts_upload_and_retains_already_received_ram() {
    let rows = upload(50_000, CODE);
    let mut m = machine(255);
    m.run_until(at(68_000), &rows, &mut ()).unwrap();
    assert_eq!(m.ram()[0x400], 0x7a);
    let before = m.ram().to_vec();
    let reset = [
        pin(68_000, Input::ResetPin(false)),
        pin(68_100, Input::ResetPin(true)),
    ];
    m.run_until(at(90_000), &reset, &mut ()).unwrap();
    assert_eq!(m.phase_name(), "boot-service");
    assert_eq!(m.peek(0xff99).unwrap(), 255);
    assert_eq!(m.ram().as_slice(), before);
    let mut loaded = state::restore_file(&m.snapshot());
    loaded.run_until(at(1_000_000), &[], &mut ()).unwrap();
    assert_eq!(loaded.retired(), 0);
}
