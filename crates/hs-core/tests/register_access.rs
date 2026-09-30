//! Expectations are transcribed separately from the production decoder.
use hs_core::{Conditions, Images, Machine, Time};

#[test]
fn actual_guest_accesses_commit_at_the_documented_exclusive_boundary() {
    // Reset: vector, two internal states, initial prefetch. MOV.B @aa:16,R0L
    // then fetches the extension and NEXT before its physical data access.
    for (address, data_states) in [
        (0xf068u16, 2),
        (0xf0d0, 2),
        (0xf0f0, 2),
        (0xffbf, 2),
        (0xffbc, 2),
        (0xf0f8, 2),
        (0xff8c, 2),
        (0xf088, 2),
        (0xc000, 2),
        (0xff91, 2),
        (0xffa7, 2),
        (0xf0e4, 3),
        (0xff9c, 3),
    ] {
        let mut rom = vec![0; 49152];
        rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
        rom[0x100..0x106].copy_from_slice(&[
            0x6a,
            0x08,
            (address >> 8) as u8,
            address as u8,
            0x40,
            0xfe,
        ]);
        let mut conditions = Conditions::default();
        conditions.clocks.main_hz = 1_000_000;
        let mut m = Machine::with_conditions(
            Images {
                firmware: &rom,
                eeprom: &[0xff; 65536],
                eeprom_status: 0,
                sensor_nonvolatile: None,
            },
            conditions,
        )
        .unwrap();
        // Use the actual rational clock representation: from_micros floors
        // timestamps, and a boundary one quantum later includes the effect.
        let boundary = hs_core::diagnostic::Clock::new(Time::ZERO, 1_000_000, 1)
            .unwrap()
            .after(10 + data_states)
            .unwrap();
        m.run_until(boundary, &[], &mut ()).unwrap();
        assert_eq!(m.statistics().bus_reads, 4, "early access {address:04x}");
        m.run_until(Time::from_raw(boundary.raw() + 1), &[], &mut ())
            .unwrap();
        assert_eq!(m.statistics().bus_reads, 5, "late access {address:04x}");
    }
}

#[test]
fn native_word_latches_keep_both_lanes_when_firmware_uses_byte_stores() {
    use hs_core::diagnostic::{cpu::WriteOrigin, mcu::Mcu};
    let mut mcu = Mcu::new(&[0; 49152], Default::default()).unwrap();
    // The Timer W counter/compare registers and AEC period register
    // accept a native word strobe. Byte reads select a lane, while byte stores
    // cannot replace either half of the retained word (REJ09B0152-0300 §20.1).
    for (address, value) in [
        (0xf0f6u16, 0x1234u16),
        (0xf0f8, 0x5678),
        (0xf0fa, 0x9abc),
        (0xf0fc, 0xdef0),
        (0xf0fe, 0x2468),
        (0xff8c, 0x1357),
    ] {
        mcu.write16(address + 1, value, Time::ZERO, &mut ())
            .unwrap();
        for (lane, expected) in value.to_be_bytes().into_iter().enumerate() {
            let a = address + lane as u16;
            assert_eq!(mcu.read8(a, Time::ZERO, &mut ()).unwrap(), expected);
            assert_eq!(mcu.peek8(a).unwrap(), expected);
            mcu.write8(a, !expected, WriteOrigin::Other, Time::ZERO, &mut ())
                .unwrap();
        }
        assert_eq!(mcu.read16(address, Time::ZERO, &mut ()).unwrap(), value);
    }
}
