//! Private retail gameplay through physical inputs and the public embedding API.
//! Layouts follow lumirth/pw at 6dc7bc09950078fa3fe0dffa4dae34e9549a99da:
//! globals.h, records.h, pw_dowsing.c and pw_battle.c.
#[path = "support/state.rs"]
mod state;
use hs_core::{Acceleration, Buttons, Event, Images, Input, Machine, Time, TimedInput};

const LEFT: u8 = 0;
const CENTER: u8 = 1;
const RIGHT: u8 = 2;

fn buttons(presses: &[(u64, u8)]) -> Vec<TimedInput> {
    presses
        .iter()
        .flat_map(|&(ms, button)| {
            [
                TimedInput {
                    at: Time::from_micros(ms * 1000),
                    input: Input::Buttons(Buttons {
                        left: button == LEFT,
                        center: button == CENTER,
                        right: button == RIGHT,
                    }),
                },
                TimedInput {
                    at: Time::from_micros((ms + 250) * 1000),
                    input: Input::Buttons(Buttons::RELEASED),
                },
            ]
        })
        .collect()
}

fn play(mut m: Machine, inputs: &[TimedInput], checkpoint_ms: u64, end_ms: u64) -> Machine {
    let mut restored = m.clone();
    let mut audio = m.audio(48_000).unwrap();
    let mut expected = vec![];
    let mut observed = vec![];
    let end = Time::from_micros(end_ms * 1000);
    m.run_until(end, inputs, &mut expected).unwrap();
    let prefix = restored
        .run_until(
            Time::from_micros(checkpoint_ms * 1000),
            inputs,
            &mut observed,
        )
        .unwrap();
    restored = state::restore_file(&restored.snapshot());
    restored
        .run_until(end, &inputs[prefix.inputs_consumed..], &mut observed)
        .unwrap();
    assert_eq!(expected, observed);
    state::assert_same_state(&m, &restored);
    assert!(expected.iter().any(|e| matches!(e, Event::NvCommit { .. })));
    let mut audible = false;
    let mut samples = |block: &[i16]| audible |= block.iter().any(|sample| *sample != 0);
    for event in expected {
        audio.event(event, &mut samples).unwrap();
    }
    audio.advance(end, &mut samples).unwrap();
    assert!(audible);
    let mut pixels = [0; 6144];
    m.display(&mut pixels);
    assert!(pixels.iter().any(|pixel| *pixel != 0));
    // DisplaySetContrast adds the board calibration to the saved adjustment.
    let configured_contrast = (m.ram()[0x29] + ((m.ram()[23] >> 3) & 15)) & 63;
    assert_eq!(m.display_contrast(), configured_contrast);
    assert_eq!(m.ram()[0x31], 0, "firmware returned to its home view");
    m
}

#[test]
#[ignore = "requires HS_FIRMWARE and HS_EEPROM; run tools/verify_retail.py"]
fn walking_dowsing_and_capture_preserve_rewards_through_restoration() {
    let firmware = std::fs::read(std::env::var("HS_FIRMWARE").expect("HS_FIRMWARE")).unwrap();
    let eeprom = std::fs::read(std::env::var("HS_EEPROM").expect("HS_EEPROM")).unwrap();
    let mut walking = Machine::new(Images {
        firmware: &firmware,
        eeprom: &eeprom,
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap();
    let motion: Vec<_> = include_str!("../../../workloads/walking.csv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split(',').collect();
            assert_eq!(fields[1], "accel");
            TimedInput {
                at: Time::from_micros(fields[0].parse().unwrap()),
                input: Input::Acceleration(Acceleration {
                    x: fields[2].parse().unwrap(),
                    y: fields[3].parse().unwrap(),
                    z: fields[4].parse().unwrap(),
                }),
            }
        })
        .collect();
    walking
        .run_until(Time::from_micros(61_000_000), &motion, &mut ())
        .unwrap();
    assert_eq!(&walking.ram()[14..16], &[0, 5]);

    // Spend three earned Watts, select the second-right patch and collect its item.
    let dowsing = play(
        walking.clone(),
        &buttons(&[
            (61000, CENTER),
            (61500, LEFT),
            (62000, CENTER),
            (62500, RIGHT),
            (63000, RIGHT),
            (63500, CENTER),
            (67000, CENTER),
        ]),
        64000,
        68500,
    );
    assert_eq!(&dowsing.ram()[14..16], &[0, 2]);
    for address in [0x0156, 0x0256] {
        assert_eq!(&dowsing.eeprom()[address + 14..address + 16], &[0, 2]);
    }
    // WalkData.items starts after its 12-byte prefix and three 16-byte Pokemon.
    assert_eq!(&dowsing.eeprom()[0xcebc..0xcebe], &[0x11, 0]);

    // Exercise the frontend's RAM editing contract to fund a separate radar run.
    // The guest owns the entry charge, battle, capture and EEPROM writes.
    walking.write_ram(0xf78e, &100u16.to_be_bytes()).unwrap();
    let capture = play(
        walking,
        &buttons(&[
            (61000, RIGHT),
            (61500, CENTER),
            (62000, RIGHT),
            (62500, CENTER),
            (64750, RIGHT),
            (65250, RIGHT),
            (65750, CENTER),
            (71500, CENTER),
            (72000, CENTER),
            (72500, CENTER),
            (73000, CENTER),
            (73500, LEFT),
            (77500, LEFT),
            (81500, CENTER),
            (93250, CENTER),
        ]),
        85000,
        94750,
    );
    assert_eq!(&capture.ram()[14..16], &[0, 90]);
    // The third course encounter occupies bytes 114..130 of Course. The first
    // captured Pokemon occupies bytes 12..28 of WalkData.
    assert_eq!(&capture.eeprom()[0xce8c..0xce9c], &eeprom[0x8f72..0x8f82]);
    assert_ne!(&capture.eeprom()[0xce8c..0xce8e], &[0, 0]);
}
