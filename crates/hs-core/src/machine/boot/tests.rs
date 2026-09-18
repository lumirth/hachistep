//! Sample the resolved TXD waveform as a UART host; no completed-byte shortcut.
use crate::{DigitalPin, Images, Input, Machine, Time, TimedInput};
fn input(us: u64, input: Input) -> TimedInput {
    TimedInput {
        at: Time::from_micros(us),
        input,
    }
}
#[test]
fn host_receives_exact_echoes_and_the_final_complete_stop_bit() {
    let code = [0x7a, 7, 0, 0, 0xff, 0x70, 0x01, 0x80, 0x40, 0xfe];
    let mut rows = vec![
        input(0, Input::ResetPin(false)),
        input(0, Input::NmiPin(false)),
        input(100, Input::ResetPin(true)),
    ];
    let mut frames = vec![(1000, 0), (20_000, 0x55), (50_000, 0), (56_000, 10)];
    frames.extend(
        code.iter()
            .enumerate()
            .map(|(i, b)| (62_000 + i as u64 * 6000, *b)),
    );
    for (start, byte) in frames {
        for bit in 0..10u64 {
            rows.push(input(
                start + bit * 1250 / 3,
                Input::DigitalPin {
                    pin: DigitalPin::P31,
                    level: Some(bit == 9 || (bit != 0 && byte & (1 << (bit - 1)) != 0)),
                },
            ));
        }
    }
    let mut m = Machine::new(Images {
        firmware: &[255; 49152],
        eeprom: &[255; 65536],
        eeprom_status: 0,
    })
    .unwrap();
    let mut used = 0;
    let mut edges = vec![(0u64, true)];
    let mut first_instruction = None;
    for us in (20..=140_000u64).step_by(20) {
        used += m
            .run_until(Time::from_micros(us), &rows[used..], &mut ())
            .unwrap()
            .inputs_consumed;
        let high = m.mcu.gpio.levels[1] & 4 != 0;
        if high != edges.last().unwrap().1 {
            edges.push((us, high));
        }
        if m.retired() != 0 && first_instruction.is_none() {
            first_instruction = Some(us);
        }
    }
    let level = |us| edges[edges.partition_point(|(at, _)| *at <= us) - 1].1;
    let mut decoded = vec![];
    let mut end = 500;
    let mut stop_end = 0;
    for &(start, high) in &edges {
        if high || start < end {
            continue;
        }
        let mut byte = 0;
        for bit in 0..8 {
            byte |= u8::from(level(start + (2 * bit + 3) * 1250 / 6)) << bit;
        }
        assert!(
            level(start + 19 * 1250 / 6),
            "missing stop after {byte:02x}"
        );
        decoded.push(byte);
        end = start + 19 * 1250 / 6;
        stop_end = start + 10 * 1250 / 3;
    }
    let expected: Vec<_> = [0, 0xaa, 0, 10]
        .into_iter()
        .chain(code)
        .chain([0xaa])
        .collect();
    assert_eq!(decoded, expected);
    // Sampling quantizes edge and retirement observations by at most 20 us.
    assert!(first_instruction.unwrap() + 20 >= stop_end);
}

#[test]
fn native_validation_checks_the_operation_after_echo_and_verify() {
    use super::{AfterSend, Boot, Receive, Sequence, Stage};
    let m = Machine::new(Images {
        firmware: &[255; 49152],
        eeprom: &[255; 65536],
        eeprom_status: 0,
    })
    .unwrap();
    let mut b = Boot::new(false);
    b.length = 0;
    b.cursor = 1024;
    for stage in [
        Stage::Send(AfterSend::Receive(Receive::Payload)),
        Stage::SendStatus(AfterSend::Receive(Receive::Payload)),
        Stage::SendByte(AfterSend::Receive(Receive::Payload)),
    ] {
        b.stage = stage;
        assert!(b.validate(Time::ZERO, &m.mcu).is_err());
    }
    b.stage = Stage::Sequence(Sequence::FlashBegin, 0);
    b.block = 6;
    assert!(b.validate(Time::ZERO, &m.mcu).is_err());
    b.stage = Stage::VerifyLow;
    b.block = 0;
    b.address = 0x800;
    assert!(b.validate(Time::ZERO, &m.mcu).is_err());
}
