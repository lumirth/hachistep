//! Physical reads and flag qualification have different serial dependencies.
#[path = "support/state.rs"]
mod state;
use hs_core::{Conditions, Images, Machine, Time};

fn store(code: &mut Vec<u8>, address: u16, value: u8) {
    code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
}
fn read_to(code: &mut Vec<u8>, address: u16, destination: u16) {
    code.extend([
        0x6a,
        8,
        (address >> 8) as u8,
        address as u8,
        0x6a,
        0x88,
        (destination >> 8) as u8,
        destination as u8,
    ]);
}
fn machine(code: &[u8]) -> Machine {
    let mut rom = vec![0; 49152];
    rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
    rom[0x100..0x100 + code.len()].copy_from_slice(code);
    let mut conditions = Conditions::default();
    conditions.clocks.main_hz = 1_000_000;
    Machine::with_conditions(
        Images {
            firmware: &rom,
            eeprom: &[255; 65536],
            eeprom_status: 0,
            sensor_nonvolatile: None,
        },
        conditions,
    )
    .unwrap()
}
fn setup(code: &mut Vec<u8>) {
    for (address, value) in [
        (0xfffb, 0x14),
        (0xf0e0, 0x8c),
        (0xf0e1, 0x40),
        (0xf0e2, 0x86),
        (0xf0e3, 0x80),
    ] {
        store(code, address, value);
    }
}
#[test]
fn live_mosi_reads_materialize_the_transmitted_bits_inside_a_byte() {
    let mut code = vec![];
    setup(&mut code);
    store(&mut code, 0xf0eb, 0x55);
    code.extend([0, 0]);
    for destination in [0xf800, 0xf801, 0xf802] {
        read_to(&mut code, 0xf0e0, destination);
    }
    code.extend([0x40, 0xfe]);
    let mut whole = machine(&code);
    let mut split = whole.clone();
    whole
        .run_until(Time::from_micros(200), &[], &mut ())
        .unwrap();
    // MOV.B absolute MMIO uses three data states; the first read falls between
    // the second launch at +7 phi ticks and the third launch at +11. Subsequent
    // reads follow the next launch at +19 and the final launch at +31. Pattern
    // 01010101 therefore supplies 1,0,1 in SSCRH's live SOL bit.
    assert_eq!(
        [0xf800, 0xf801, 0xf802].map(|a| whole.peek(a).unwrap() & 0x10),
        [0x10, 0, 0x10]
    );
    for quarter in 1..=800 {
        split
            .run_until(
                Time::from_raw(Time::from_micros(quarter).raw() / 4),
                &[],
                &mut (),
            )
            .unwrap();
        if quarter % 131 == 0 {
            split = state::restore_file(&split.snapshot());
        }
    }
    state::assert_same_state(&whole, &split);
}
#[test]
fn an_early_status_read_cannot_qualify_a_future_tend_clear() {
    let mut code = vec![];
    setup(&mut code);
    store(&mut code, 0xf0eb, 0x55);
    read_to(&mut code, 0xf0e4, 0xf800); // TDRE, before frame completion.
    for _ in 0..20 {
        code.extend([0, 0]);
    }
    store(&mut code, 0xf0e4, 0);
    read_to(&mut code, 0xf0e4, 0xf801);
    store(&mut code, 0xf0e4, 0);
    read_to(&mut code, 0xf0e4, 0xf802);
    code.extend([0x40, 0xfe]);
    let mut m = machine(&code);
    m.run_until(Time::from_micros(300), &[], &mut ()).unwrap();
    assert_eq!(m.peek(0xf800).unwrap(), 4, "the early read sees TDRE only");
    // Clearing the observed TDRE can queue another holding-register load. TEND
    // still requires its own read qualification, independently of that load.
    assert_eq!(
        [0xf800, 0xf801, 0xf802].map(|a| m.peek(a).unwrap() & 8),
        [0, 8, 0],
        "TEND needs its own read qualification before a status write clears it"
    );
}

#[test]
fn a_fault_after_status_polling_materializes_prior_edges() {
    let mut code = vec![];
    setup(&mut code);
    store(&mut code, 0xf0eb, 0xa5);
    read_to(&mut code, 0xf0e4, 0xf800);
    code.extend([0, 0, 0x57, 0xff]); // Invalid TRAPA immediate encoding.
    let mut whole = machine(&code);
    let mut split = whole.clone();
    let error = whole
        .run_until(Time::from_micros(200), &[], &mut ())
        .unwrap_err();
    assert!(matches!(error, hs_core::Error::Decode { .. }));
    let mut observed = None;
    for quarter in 1..=800 {
        if let Err(error) = split.run_until(
            Time::from_raw(Time::from_micros(quarter).raw() / 4),
            &[],
            &mut (),
        ) {
            observed = Some(error);
            break;
        }
    }
    assert_eq!(observed, Some(error));
    state::assert_same_state(&whole, &split);
    let restored = state::restore_file(&whole.snapshot());
    state::assert_same_state(&whole, &restored);
}
