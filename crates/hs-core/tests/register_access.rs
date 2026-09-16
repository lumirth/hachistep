//! Expectations are transcribed separately from the production decoder.
use hs_core::{cpu::Width, mcu::Mcu, Conditions, Images, Machine, Time};

#[test]
fn complete_documented_register_width_and_timing_map() {
    let mut count = 0;
    for line in include_str!("../../../conformance/spec/register_access.tsv").lines() {
        if line.starts_with('#') || line.is_empty() { continue; }
        let row: Vec<_> = line.split_whitespace().collect();
        assert_eq!(row.len(), 5, "invalid expectation row: {line}");
        let address = u16::from_str_radix(row[0], 16).unwrap();
        let bits: u8 = row[2].parse().unwrap();
        let states: u64 = row[3].parse().unwrap();
        let width = if bits == 16 { Width::Word } else { Width::Byte };
        assert_eq!(Mcu::access_states(address, width), states, "{} @{} p.{}",row[1],row[0],row[4]);
        assert_eq!(Mcu::native_word(address), bits == 16, "{} width", row[1]);
        count += 1;
    }
    assert_eq!(count, 95, "a register expectation was accidentally lost");
}

#[test]
fn actual_guest_accesses_commit_at_the_documented_exclusive_boundary() {
    // MOV.B @aa:16,R0L. Two 2-state instruction fetches precede the data
    // access; no native register peek substitutes for the CPU's bus operation.
    for (address, data_states) in [(0xf068u16,2),(0xf0d0,2),(0xf0f0,2),
                                   (0xffbf,2),(0xff91,2),(0xffa7,2),(0xf0e4,3),(0xff9c,3)] {
        let mut rom = vec![0;49152];
        rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
        rom[0x100..0x106].copy_from_slice(&[0x6a,0x08,(address>>8) as u8,address as u8,0x40,0xfe]);
        let mut conditions = Conditions::default();
        conditions.clocks.main_hz = 1_000_000;
        let mut m = Machine::with_conditions(Images {firmware:&rom,eeprom:&[0xff;65536],eeprom_status:0},conditions).unwrap();
        // Use the actual rational clock representation: from_micros floors
        // timestamps, and a boundary one quantum later includes the effect.
        let boundary = hs_core::time::Clock::new(Time::ZERO,1_000_000,1).unwrap().after(4+data_states).unwrap();
        m.run_until(boundary,&[],&mut ()).unwrap();
        assert_eq!(m.statistics().bus_reads,2,"early access {address:04x}");
        m.run_until(Time::from_raw(boundary.raw()+1),&[],&mut ()).unwrap();
        assert_eq!(m.statistics().bus_reads,3,"late access {address:04x}");
    }
}
