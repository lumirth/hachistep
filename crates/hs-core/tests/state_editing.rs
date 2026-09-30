use hs_core::{Images, Machine, Snapshot, Time};

fn machine() -> Machine {
    let mut firmware = vec![0; 49_152];
    firmware[..2].copy_from_slice(&0x100u16.to_be_bytes());
    // Repeatedly copy the byte at F780 to F781.
    firmware[0x100..0x10a]
        .copy_from_slice(&[0x6a, 0x08, 0xf7, 0x80, 0x6a, 0x88, 0xf7, 0x81, 0x40, 0xf6]);
    Machine::new(Images {
        firmware: &firmware,
        eeprom: &[255; 65_536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap()
}

#[test]
fn host_edits_are_visible_to_firmware_and_survive_restoration() {
    let mut m = machine();
    m.write_ram(0xf780, &[42]).unwrap();
    m.run_until(Time::from_micros(30), &[], &mut ()).unwrap();
    assert_eq!(m.peek(0xf781).unwrap(), 42);
    let now = m.now();
    let registers = m.registers().clone();
    m.write_ram(0xf780, &[99]).unwrap();
    m.write_eeprom(0xfffe, &[12, 34]).unwrap();
    assert_eq!(m.now(), now);
    assert_eq!(m.registers(), &registers);
    let captured = Snapshot::decode(&m.snapshot().encode().unwrap()).unwrap();
    let mut restored = Machine::from_snapshot(&captured);
    for session in [&mut m, &mut restored] {
        session
            .run_until(Time::from_micros(60), &[], &mut ())
            .unwrap();
        assert_eq!(session.peek(0xf781).unwrap(), 99);
        assert_eq!(&session.eeprom()[0xfffe..], &[12, 34]);
    }
    assert_eq!(m.snapshot().encode(), restored.snapshot().encode());
}

#[test]
fn invalid_ranges_fail_without_partial_writes_or_machine_faults() {
    let mut m = machine();
    let before = m.snapshot();
    for address in [0, 0xf77f, 0xff7f, 0xff80, 0xffff] {
        assert!(m.write_ram(address, &[1, 2]).is_err());
        assert_eq!(m.snapshot(), before);
    }
    assert!(m.write_eeprom(0xffff, &[1, 2]).is_err());
    assert_eq!(m.snapshot(), before);
    assert!(m.fault().is_none());
    m.write_ram(0xff7f, &[1]).unwrap();
    m.write_eeprom(0xffff, &[2]).unwrap();
    assert_eq!(m.ram()[2047], 1);
    assert_eq!(m.eeprom()[65535], 2);
}

#[test]
fn edited_ram_can_execute_without_a_firmware_specific_path() {
    let mut firmware = vec![0; 49_152];
    firmware[..2].copy_from_slice(&0xf780u16.to_be_bytes());
    let mut m = Machine::new(Images {
        firmware: &firmware,
        eeprom: &[255; 65_536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap();
    m.write_ram(0xf780, &[0xf8, 42, 0x40, 0xfc]).unwrap();
    m.run_until(Time::from_micros(20), &[], &mut ()).unwrap();
    assert_eq!(m.registers().er[0] & 255, 42);
    m.write_ram(0xf781, &[99]).unwrap();
    m.run_until(Time::from_micros(40), &[], &mut ()).unwrap();
    assert_eq!(m.registers().er[0] & 255, 99);
}
