#[path = "support/flash.rs"]
mod guest;
#[path = "support/state.rs"]
mod state;
use hs_core::{Conditions, Event, Frequencies, Images, Input, Machine, NvDomain, Time, TimedInput};

#[test]
fn ram_programming_and_verify_survive_partition_and_mid_pulse_restoration() {
    let mut whole = guest::machine();
    let mut split = whole.clone();
    let (mut a, mut b) = (vec![], vec![]);
    whole
        .run_until(Time::from_micros(12000), &[], &mut a)
        .unwrap();
    let mut us = 0;
    while us < 12000 {
        us = (us + 137).min(12000);
        split.run_until(Time::from_micros(us), &[], &mut b).unwrap();
        let before = split.snapshot();
        let _ = split.firmware();
        let _ = split.peek(0x9000).unwrap();
        assert_eq!(
            before,
            split.snapshot(),
            "projection cannot complete or protect a pulse"
        );
        split = state::restore_file(&before);
    }
    assert_eq!(whole.firmware()[0x9000], 0);
    assert_ne!(&whole.ram()[0x180..0x182], &[0xff, 0xff]);
    state::assert_same_state(&whole, &split);
    assert_eq!(a, b);
}

#[test]
fn power_loss_and_mcu_reset_retain_partial_programming() {
    for input in [Input::Power(false), Input::ResetPin(false)] {
        let mut m = guest::machine();
        m.run_until(Time::from_micros(4000), &[], &mut ()).unwrap();
        let projected = m.firmware();
        assert_ne!(projected[0x9000], 0xff);
        let before = m.snapshot();
        let event = TimedInput {
            at: Time::from_micros(4000),
            input,
        };
        m.run_until(Time::from_micros(12000), &[event], &mut ())
            .unwrap();
        assert_eq!(m.firmware(), projected);
        let mut replay = Machine::from_snapshot(&before);
        replay
            .run_until(Time::from_micros(12000), &[event], &mut ())
            .unwrap();
        state::assert_same_state(&m, &replay);
    }
}

#[test]
fn a_ram_trap_protects_flash_at_exception_admission_after_the_completed_pulse_prefix() {
    let mut rom = vec![0xff; 49_152];
    rom[..2].copy_from_slice(&0xf980_u16.to_be_bytes());
    let mut code = vec![0x79, 7, 0xff, 0x70];
    let store = |code: &mut Vec<u8>, address: u16, value| {
        code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
    };
    store(&mut code, 0xf02b, 0x80); // Expose the flash registers.
    store(&mut code, 0xf020, 0x40); // SWE; the following access is after its 1µs settling.
    store(&mut code, 0x9000, 0); // Load one byte of the program latch.
    store(&mut code, 0xf020, 0x50); // PSU.
    code.extend([0; 64]); // 32 NOPs provide 64µs for the 50µs setup interval.
    store(&mut code, 0xf020, 0x51); // P rises at 114µs.
    code.extend([0x79, 2, 1, 0xf4, 0x1b, 0x52, 0x46, 0xfc]); // 500 DEC/BNE iterations.
    let trap_pc = 0xf980 + code.len() as u16;
    code.extend([0x57, 0, 0, 0]); // TRAPA #0, then its discarded NEXT word.
    let mut whole = Machine::with_conditions(
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
    .unwrap();
    whole.write_ram(0xf980, &code).unwrap();
    let mut split = whole.clone();
    // At 1MHz: P rises at 114µs, immediate MOV.W costs 4 states, each DEC/BNE
    // iteration costs 6, and TRAPA's discarded NEXT costs 2 before admission.
    // Protection is at 3120µs, before its internal wait and exception stack writes.
    let admitted = Time::from_micros(3120);
    let end = Time::from_micros(3127); // Before the protected flash vector read.
    let (mut expected, mut observed) = (vec![], vec![]);
    whole.run_until(end, &[], &mut expected).unwrap();
    for us in (1..3127)
        .step_by(37)
        .chain([3119, 3120, 3121, 3125, 3127])
        .collect::<std::collections::BTreeSet<_>>()
    {
        split
            .run_until(Time::from_micros(us), &[], &mut observed)
            .unwrap();
        if us == 1111 || us >= 3119 {
            split = state::restore_file(&split.snapshot());
        }
    }
    let interrupted: Vec<_> = expected
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::NvInterrupted {
                    domain: NvDomain::InternalFlash,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(
        interrupted,
        [&Event::NvInterrupted {
            at: admitted,
            domain: NvDomain::InternalFlash,
            address: 0x9000,
            length: 128,
        }]
    );
    assert_eq!(whole.peek(0xf021).unwrap() & 0x80, 0x80);
    // docs/accuracy/flash.md defines the local exposure/sense distribution.
    // 3006µs crosses five normal-read thresholds at 9000, leaving bits 7/6/1.
    // This checks retained model progress, not a measured silicon bit pattern.
    assert_eq!(whole.firmware()[0x9000], 0xc2);
    assert_eq!(&whole.ram()[0x7ee..0x7f0], &(trap_pc + 2).to_be_bytes());
    assert_eq!(&whole.ram()[0x7ec..0x7ee], &[0x84, 0x84]);
    assert_eq!(whole.interrupt_entries(), 1);
    assert_eq!(observed, expected);
    state::assert_same_state(&whole, &split);
}
