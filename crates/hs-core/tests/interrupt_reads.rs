//! Reads that remove a peripheral condition must also remove its IRQ request.
#[path = "support/state.rs"]
mod state;

use hs_core::{
    AnalogPin, Conditions, DigitalPin, Duration, Frequencies, Images, Input, Machine, Time,
    TimedInput,
};

fn byte(code: &mut Vec<u8>, address: u16, value: u8) {
    code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
}

fn machine(code: &[u8], vector: u8, handler: &[u8]) -> Machine {
    let mut rom = vec![0; 49_152];
    rom[..2].copy_from_slice(&0x100_u16.to_be_bytes());
    let offset = usize::from(vector) * 2;
    rom[offset..offset + 2].copy_from_slice(&0x400_u16.to_be_bytes());
    rom[0x100..0x100 + code.len()].copy_from_slice(code);
    rom[0x400..0x400 + handler.len()].copy_from_slice(handler);
    Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[0xff; 65_536],
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
fn a_comparator_read_at_the_response_instant_removes_only_the_new_request() {
    let mut code = vec![0x79, 7, 0xff, 0x70];
    byte(&mut code, 0xfffb, 6);
    byte(&mut code, 0xf0dc, 0xc0); // Comparator 0, internal reference, IRQ enabled.
    for _ in 0..32 {
        code.extend([0, 0]);
    }
    code.extend([0x6a, 0x08, 0xf0, 0xde, 0xf8, 0xa5]); // Arm at the low baseline.
    for _ in 0..64 {
        code.extend([0, 0]);
    }
    let read_pc = 0x100 + code.len() as u16;
    code.extend([0x6a, 0x08, 0xf0, 0xde, 0x06, 0x7f]);
    for _ in 0..64 {
        code.extend([0, 0]);
    }
    code.extend([0x6a, 0x88, 0xf7, 0x80, 0x40, 0xfe]);
    let handler = [
        0xf9, 1, 0x6a, 0x89, 0xf7, 0x81, 0x6a, 0x09, 0xf0,
        0xde, // Qualify the older flag before clearing it.
        0xf9, 0, 0x6a, 0x89, 0xf0, 0xde, 0x56, 0x70,
    ];
    let low = TimedInput {
        at: Time::ZERO,
        input: Input::AnalogPin {
            pin: AnalogPin::Pb4,
            millivolts: Some(0),
        },
    };
    // Find the physical read instant from its register result. This aligns the
    // collision without making an absolute instruction-duration claim.
    let mut calibration = machine(&code, 21, &handler);
    let calibration_inputs = [low];
    let mut read_at = None;
    let mut reading = false;
    for us in 1..=1000 {
        let end = Time::from_raw(Time::from_micros(us).raw() + 1);
        calibration
            .run_until(
                end,
                if us == 1 { &calibration_inputs } else { &[] },
                &mut (),
            )
            .unwrap();
        reading |= calibration.instruction_pc() == read_pc;
        if reading && calibration.registers().er[0] as u8 == 0 {
            read_at = Some(Time::from_micros(us));
            break;
        }
    }
    let read_at = read_at.expect("guest reached its second comparator read");
    for coincident in [false, true] {
        let due = Time::from_raw(read_at.raw() - u128::from(!coincident));
        let transition = Time::from_raw(due.raw() - Duration::from_micros(15).raw());
        let inputs = [
            low,
            TimedInput {
                at: transition,
                input: Input::AnalogPin {
                    pin: AnalogPin::Pb4,
                    millivolts: Some(2000),
                },
            },
        ];
        let mut whole = machine(&code, 21, &handler);
        let mut split = whole.clone();
        let (mut expected, mut observed) = (vec![], vec![]);
        let end = Time::from_micros(1000);
        whole.run_until(end, &inputs, &mut expected).unwrap();
        let before = Time::from_raw(read_at.raw() - 1);
        let consumed = split
            .run_until(before, &inputs, &mut observed)
            .unwrap()
            .inputs_consumed;
        split = state::restore_file(&split.snapshot());
        split
            .run_until(end, &inputs[consumed..], &mut observed)
            .unwrap();
        assert_eq!(whole.peek(0xf781).unwrap(), u8::from(!coincident));
        assert_eq!(whole.interrupt_entries(), u64::from(!coincident));
        assert_eq!(
            whole.peek(0xf780).unwrap() & 0x11,
            if coincident { 1 } else { 0x11 }
        );
        assert_eq!(whole.peek(0xf0de).unwrap() & 0x10, 0);
        assert_eq!(observed, expected);
        state::assert_same_state(&whole, &split);
    }
}

#[test]
fn reading_sci_data_while_masked_removes_the_request_before_unmasking() {
    let pin = |us, pin, level| TimedInput {
        at: Time::from_micros(us),
        input: Input::DigitalPin {
            pin,
            level: Some(level),
        },
    };
    let mut inputs = vec![
        pin(0, DigitalPin::P30, true),
        pin(0, DigitalPin::P31, false),
    ];
    for bit in 0..8 {
        inputs.extend([
            pin(500 + 100 * bit, DigitalPin::P31, 0x3c & (1 << bit) != 0),
            pin(500 + 100 * bit, DigitalPin::P30, false),
            pin(550 + 100 * bit, DigitalPin::P30, true),
        ]);
    }
    for consume in [true, false] {
        let mut code = vec![0x79, 7, 0xff, 0x70]; // Stack for the control's SCI exception.
        for (address, value) in [
            (0xfffa, 0x43),
            (0xff91, 0xd0),
            (0xff98, 0x80), // Synchronous format.
            (0xff99, 0),
            (0xff9a, 0x52), // Receive, receive interrupt, external SCK.
        ] {
            byte(&mut code, address, value);
        }
        // CCR.I stays set while polling SSR.RDRF.
        code.extend([0x6a, 0x08, 0xff, 0x9c, 0xe8, 0x40, 0x47, 0xf8]);
        if consume {
            code.extend([0x6a, 0x08, 0xff, 0x9d]);
        }
        // RDR clears RDRF. The following register-only sequence must
        // not accept the removed SCI request when ANDC clears CCR.I.
        code.extend([0x06, 0x7f, 0, 0, 0, 0, 0x6a, 0x88, 0xf7, 0x80, 0x40, 0xfe]);
        let handler = [
            0xf9, 1, 0x6a, 0x89, 0xf7, 0x81, // Mark vector 37.
            0x6a, 0x09, 0xff, 0x9d, // Drain RDR to prevent repeated requests.
            0x56, 0x70,
        ];
        let mut whole = machine(&code, 37, &handler);
        let mut split = whole.clone();
        let end = Time::from_micros(1500);
        let (mut expected, mut observed) = (vec![], vec![]);
        whole.run_until(end, &inputs, &mut expected).unwrap();
        let mut consumed = 0;
        for us in (1..1500).step_by(7).chain([1500]) {
            consumed += split
                .run_until(Time::from_micros(us), &inputs[consumed..], &mut observed)
                .unwrap()
                .inputs_consumed;
            if us == 1247 {
                split = state::restore_file(&split.snapshot());
            }
        }
        assert_eq!(whole.peek(0xf781).unwrap(), u8::from(!consume));
        assert_eq!(whole.interrupt_entries(), u64::from(!consume));
        assert_eq!(
            whole.peek(0xf780).unwrap(),
            if consume { 0x3c } else { 0x40 }
        );
        assert_eq!(whole.peek(0xff9c).unwrap() & 0x40, 0);
        assert_eq!(observed, expected);
        state::assert_same_state(&whole, &split);
    }
}
