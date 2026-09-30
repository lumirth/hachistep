//! Target §3.8.4: enable clearing retains one admission opportunity; I still masks it.
#[path = "support/state.rs"]
mod state;
use hs_core::{Conditions, Frequencies, Images, Machine, Time};

#[test]
fn disabled_enable_expires_at_its_instruction_boundary_and_survives_snapshot_before_it() {
    for masked in [false, true] {
        let mut code = vec![
            0x79, 7, 0xff, 0x70, // SP
            0xf8, 0x44, 0x6a, 0x88, 0xff, 0xfb, // Timer W gate
            0xf8, 1, 0x6a, 0x88, 0xf0, 0xf2, // IMIEA
            0x79, 1, 0, 6, 0x6b, 0x81, 0xf0, 0xf8, // GRA=6, compare at phi 7
            0x79, 0, 0xf0, 0xf2, 0xf9, 0x80,
        ];
        if !masked {
            code.extend([0x06, 0x7f, 0, 0]);
        }
        code.extend([0x6a, 0x89, 0xf0, 0xf0, 0x7d, 0, 0x72, 0]); // start; BCLR enable
        let return_pc = 0x100 + code.len() as u16;
        if masked {
            code.extend([0x06, 0x7f, 0, 0]);
        }
        code.extend([0x40, 0xfe]);
        let mut rom = vec![0; 49152];
        rom[..2].copy_from_slice(&0x100_u16.to_be_bytes());
        rom[70..72].copy_from_slice(&0x200_u16.to_be_bytes());
        rom[0x100..0x100 + code.len()].copy_from_slice(&code);
        let handler = [
            0x6a, 8, 0xf0, 0xf3, 0x6a, 0x88, 0xf8, 0, 0x6a, 8, 0xf0, 0xf2, 0x6a, 0x88, 0xf8, 1,
            0xf8, 0, 0x6a, 0x88, 0xf0, 0xf3, 0x56, 0x70,
        ];
        rom[0x200..0x200 + handler.len()].copy_from_slice(&handler);
        let mut whole = Machine::with_conditions(
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
        .unwrap();
        let mut split = whole.clone();
        let (mut a, mut b) = (vec![], vec![]);
        whole
            .run_until(Time::from_micros(200), &[], &mut a)
            .unwrap();
        for half_us in 1..=400 {
            split
                .run_until(
                    Time::from_raw(Time::from_micros(half_us).raw() / 2),
                    &[],
                    &mut b,
                )
                .unwrap();
            split = state::restore_file(&split.snapshot());
        }
        assert_eq!(whole.interrupt_entries(), u64::from(!masked));
        assert_eq!(
            &whole.ram()[0x80..0x82],
            if masked { &[0, 0] } else { &[0x71, 0x70] }
        );
        if !masked {
            assert_eq!(&whole.ram()[0x7ee..0x7f0], &return_pc.to_be_bytes());
        }
        state::assert_same_state(&whole, &split);
        assert_eq!(a, b);
    }
}
