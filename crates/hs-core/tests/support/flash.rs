use hs_core::{Conditions, Frequencies, Images, Machine};

/// Original RAM routine with a six-millisecond pulse and subsequent verify.
pub fn machine() -> Machine {
    let mut body = Vec::new();
    let write = |body: &mut Vec<u8>, a: u16, v| {
        body.extend([0xf8, v, 0x6a, 0x88, (a >> 8) as u8, a as u8]);
    };
    write(&mut body, 0xf02b, 0x80);
    write(&mut body, 0xf020, 0x40);
    write(&mut body, 0x9000, 0);
    write(&mut body, 0xf020, 0x50);
    body.extend([0x79, 2, 0, 10, 0x1b, 0x52, 0x46, 0xfc]);
    write(&mut body, 0xf020, 0x51);
    body.extend([0x79, 2, 3, 0xe8, 0x1b, 0x52, 0x46, 0xfc]);
    write(&mut body, 0xf020, 0x50);
    body.extend([0; 6]);
    write(&mut body, 0xf020, 0x40);
    body.extend([0; 6]);
    write(&mut body, 0xf020, 0x44);
    body.extend([0; 6]);
    write(&mut body, 0x9000, 0xff);
    body.extend([0; 4]);
    body.extend([0x6b, 0, 0x90, 0, 0x6b, 0x80, 0xf9, 0]);
    write(&mut body, 0xf020, 0x40);
    body.extend([0x40, 0xfe]);
    let length = body.len() as u16;
    let loader = [
        0x79,
        7,
        0xff,
        0x70,
        0x79,
        5,
        4,
        0,
        0x79,
        6,
        0xf9,
        0x80,
        0x79,
        4,
        (length >> 8) as u8,
        length as u8,
        0x7b,
        0xd4,
        0x59,
        0x8f,
        0x5a,
        0,
        0xf9,
        0x80,
    ];
    let mut rom = vec![0xff; 49152];
    rom[..2].copy_from_slice(&[1, 0]);
    rom[0x100..0x100 + loader.len()].copy_from_slice(&loader);
    rom[0x400..0x400 + body.len()].copy_from_slice(&body);
    Machine::with_conditions(
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
    .unwrap()
}
