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
            },
            conditions,
        )
        .unwrap();
        // Use the actual rational clock representation: from_micros floors
        // timestamps, and a boundary one quantum later includes the effect.
        let boundary = hs_core::time::Clock::new(Time::ZERO, 1_000_000, 1)
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
