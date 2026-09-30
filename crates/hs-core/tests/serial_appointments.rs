#[path = "support/state.rs"]
mod state;
use hs_core::{Event, Images, Machine, Time};
fn machine() -> Machine {
    // Keep LCD CS active and stream a changing byte through the physical SSU.
    // Timer B1 reloads every phi/64 edge. Its overflow appointments can
    // coincide with raw SSU edges before a complete serial byte is ready.
    let mut code = vec![0x07, 0x80];
    for (address, value) in [
        (0xffb1u16, 0x10),
        (0xffb1, 0),
        (0xfffa, 6),
        (0xfffb, 0x10),
        (0xffd4, 7),
        (0xffdc, 1),
        (0xffe4, 7),
        (0xffec, 1),
        (0xf0d0, 0x83),
        (0xf0d1, 0xff),
        (0xf0d0, 0xc3),
        (0xf0e0, 0x8c),
        (0xf0e2, 0x86),
        (0xf0e4, 0xff),
        (0xf0e3, 0x80),
        (0xffd4, 6),
    ] {
        code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
    }
    code.extend([0xf9, 0x25, 0x79, 6, 4, 0]);
    let start = code.len();
    code.extend([0x0c, 0x98, 0x6a, 0x88, 0xf0, 0xeb]);
    code.extend([0x6a, 8, 0xf0, 0xe4, 0xe8, 8, 0x47, 0xf8]);
    code.extend([0x6a, 8, 0xf0, 0xe9, 0x0a, 9, 0x1b, 0x56]);
    let displacement = (start as isize - code.len() as isize - 2) as u8;
    code.extend([0x46, displacement, 0x01, 0x80, 0x40, 0xfc]);
    let mut flash = vec![0; 49152];
    flash[..2].copy_from_slice(&0x100u16.to_be_bytes());
    flash[0x100..0x100 + code.len()].copy_from_slice(&code);
    Machine::new(Images {
        firmware: &flash,
        eeprom: &[255; 65536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap()
}
#[test]
fn serial_bytes_survive_unrelated_device_appointments() {
    let mut m = machine();
    let mut events = Vec::new();
    m.run_until(Time::from_micros(100_000), &[], &mut events)
        .unwrap();
    let writes: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::LcdWrite {
                page,
                column_byte,
                value,
                ..
            } => Some((*page, *column_byte, *value)),
            _ => None,
        })
        .collect();
    let expected: Vec<_> = (0..1024)
        .map(|i| (0, (i % 256) as u16, 0x25u8.wrapping_add(i as u8)))
        .collect();
    assert_eq!(writes.len(), expected.len());
    for (index, (actual, expected)) in writes.iter().zip(expected).enumerate() {
        assert_eq!(*actual, expected, "LCD byte {index}");
    }
    assert!(m.sleeping());
    let page: Vec<_> = (0..256).map(|i| 0x25u8.wrapping_add(i as u8)).collect();
    assert_eq!(&m.lcd_ram()[..256], page);

    // Restore just before a coincident raw edge, then partition the remaining
    // interval across serial bits and device appointments.
    let mut split = machine();
    let mut observed = Vec::new();
    split
        .run_until(Time::from_micros(10_416), &[], &mut observed)
        .unwrap();
    split = state::restore_file(&split.snapshot());
    let mut at = 10_416;
    while at < 100_000 {
        at = (at + 113).min(100_000);
        split
            .run_until(Time::from_micros(at), &[], &mut observed)
            .unwrap();
    }
    assert_eq!(events, observed);
    state::assert_same_state(&m, &split);
}

#[test]
fn stopping_on_a_completed_lcd_byte_preserves_serial_progress_and_restoration() {
    use hs_core::StopReason;
    use std::ops::ControlFlow;

    let end = Time::from_micros(100_000);
    let mut whole = machine();
    let mut expected = Vec::new();
    whole.run_until(end, &[], &mut expected).unwrap();

    let mut split = machine();
    let mut observed = Vec::new();
    let mut stopped_at = None;
    let result = split
        .run_until(end, &[], &mut |event| {
            observed.push(event);
            if let Event::LcdWrite { at, value, .. } = event {
                assert_eq!(value, 0x25);
                stopped_at = Some(at);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .unwrap();
    assert_eq!(result.reason, StopReason::Output);
    assert_eq!(result.now.raw(), stopped_at.unwrap().raw() + 1);

    // The stop finishes the electrical instant, including shift-register and
    // counter state, even though the callback observes only the LCD consequence.
    let mut prefix = machine();
    let mut prefix_events = Vec::new();
    prefix
        .run_until(result.now, &[], &mut prefix_events)
        .unwrap();
    assert_eq!(observed, prefix_events);
    state::assert_same_state(&prefix, &split);

    split = state::restore_file(&split.snapshot());
    let mut repeated = Vec::new();
    split.run_until(result.now, &[], &mut repeated).unwrap();
    assert!(repeated.is_empty());
    split.run_until(end, &[], &mut observed).unwrap();
    assert_eq!(observed, expected);
    state::assert_same_state(&whole, &split);
}
