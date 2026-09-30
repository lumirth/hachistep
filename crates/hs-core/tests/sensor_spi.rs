#[path = "support/state.rs"]
mod state;
use hs_core::{Images, Machine, Time};

fn store(code: &mut Vec<u8>, address: u16, value: u8) {
    code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
}
fn send(code: &mut Vec<u8>, value: u8) {
    store(code, 0xf0eb, value);
    code.extend([0x6a, 0x08, 0xf0, 0xe4, 0xe8, 8, 0x47, 0xf8]); // Wait for TEND.
    code.extend([0x6a, 0x08, 0xf0, 0xe9]); // Consume SSRDR and clear RDRF.
}
fn session(mode: u8) -> Machine {
    let mut code = vec![0x79, 7, 0xff, 0x80];
    for (address, value) in [
        (0xfffb, 0x14),
        (0xf0e0, 0x8c),
        (0xf0e1, 0x40),
        (0xf0e2, mode),
        (0xf0e3, 0xc0),
        (0xffe4, 7),
        (0xffd4, 5),
        (0xf087, 8),
        (0xffec, 1),
        (0xffdc, 1),
    ] {
        store(&mut code, address, value);
    }
    store(&mut code, 0xffdc, 0);
    // Bosch's SPI write protocol consumes address/data pairs, including
    // a second pair without raising chip select.
    for byte in [0x0c, 0x20, 0x0d, 2] {
        send(&mut code, byte);
    }
    store(&mut code, 0xffdc, 1);
    store(&mut code, 0xffdc, 0);
    send(&mut code, 0x8c);
    for address in [0xf800, 0xf801] {
        send(&mut code, 0);
        code.extend([0x6a, 0x88, (address >> 8) as u8, address as u8]);
    }
    store(&mut code, 0xffdc, 1);
    // A control write must take effect at its last physical bit, before the
    // next transaction; sleeping serial reads are undriven.
    store(&mut code, 0xffdc, 0);
    send(&mut code, 0x0a);
    send(&mut code, 1);
    store(&mut code, 0xffdc, 1);
    store(&mut code, 0xffdc, 0);
    send(&mut code, 0x80);
    send(&mut code, 0);
    code.extend([0x6a, 0x88, 0xf8, 2]);
    store(&mut code, 0xffdc, 1);
    code.extend([0x40, 0xfe]);
    let mut firmware = vec![0; 49152];
    firmware[..2].copy_from_slice(&0x100_u16.to_be_bytes());
    firmware[0x100..0x100 + code.len()].copy_from_slice(&code);
    Machine::new(Images {
        firmware: &firmware,
        eeprom: &[0xff; 65536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap()
}

#[test]
fn spi_byte_effects_survive_quarter_microsecond_calls_and_native_restoration() {
    // These SSMR encodings select valid mode-3 and mode-0 sensor transfers.
    // Bosch BMA150 Rev1.6 §4.1 supplies the byte-level expectations.
    for mode in [0x86, 0xe6] {
        let mut whole = session(mode);
        let mut split = whole.clone();
        let end = Time::from_micros(1800);
        let (mut expected, mut observed) = (Vec::new(), Vec::new());
        whole.run_until(end, &[], &mut expected).unwrap();
        for quarter in 1..=7200 {
            let at = Time::from_raw(Time::from_micros(quarter).raw() / 4);
            split.run_until(at, &[], &mut observed).unwrap();
            if quarter % 127 == 0 {
                split = state::restore_file(&split.snapshot());
            }
        }
        assert_eq!(expected, observed, "SSMR={mode:02x}");
        state::assert_same_state(&whole, &split);
        let values = [0xf800, 0xf801, 0xf802].map(|address| whole.peek(address).unwrap());
        assert_eq!(values, [0x20, 2, 0xff], "SSMR={mode:02x}");
    }
}
