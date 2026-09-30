//! Optional peer exchange using private firmware and save images.
use hs_core::{Buttons, Duration, Event, Images, Input, Machine, Output, Time, TimedInput};
use std::{collections::VecDeque, ops::ControlFlow};

const RECORDS: usize = 0xdc00;
const RECORD_SIZE: usize = 548;
const ID_OFFSET: usize = 8;

fn image(source: &[u8], identity: u8) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let id = [identity; 40];
    for start in [0x83, 0x183] {
        bytes[start..start + 40].copy_from_slice(&id);
        checksum(&mut bytes, start, 40);
    }
    for start in [0xed, 0x1ed] {
        assert_eq!(bytes[start + 91] & 3, 3, "save needs a walking Pokemon");
        bytes[start + 16..start + 56].copy_from_slice(&id);
        checksum(&mut bytes, start, 104);
    }
    bytes[0xcc00 + ID_OFFSET..0xcc00 + ID_OFFSET + 40].copy_from_slice(&id);
    bytes[RECORDS..RECORDS + RECORD_SIZE * 11].fill(0xff);
    bytes
}

fn checksum(bytes: &mut [u8], start: usize, length: usize) {
    bytes[start + length] = bytes[start..start + length]
        .iter()
        .fold(1u8, |sum, &byte| sum.wrapping_add(byte));
}

struct Channel {
    incoming: VecDeque<TimedInput>,
    transitions: usize,
}
impl Output for Channel {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        if let Event::Infrared { at, emitting } = event {
            self.incoming.push_back(TimedInput {
                at: at.checked_add(Duration::from_micros(1)).unwrap(),
                input: Input::InfraredLevel(emitting),
            });
            self.transitions += 1;
        }
        ControlFlow::Continue(())
    }
}

fn buttons(offset: u64) -> VecDeque<TimedInput> {
    [4_500_000, 4_750_000, 5_000_000, 5_250_000]
        .into_iter()
        .enumerate()
        .map(|(i, at)| TimedInput {
            at: Time::from_micros(at + offset),
            input: Input::Buttons(Buttons {
                center: i % 2 == 0,
                ..Buttons::RELEASED
            }),
        })
        .collect()
}

fn advance(
    machine: &mut Machine,
    end: Time,
    buttons: &mut VecDeque<TimedInput>,
    incoming: &mut VecDeque<TimedInput>,
    outgoing: &mut Channel,
) {
    let mut inputs = Vec::new();
    for queue in [buttons, incoming] {
        while queue.front().is_some_and(|input| input.at < end) {
            inputs.push(queue.pop_front().unwrap());
        }
    }
    inputs.sort_by_key(|input| input.at);
    machine.run_until(end, &inputs, outgoing).unwrap();
}

#[test]
#[ignore = "requires HS_FIRMWARE and HS_EEPROM with a walking Pokemon"]
fn independent_walkers_complete_a_peer_exchange() {
    let firmware = std::fs::read(std::env::var("HS_FIRMWARE").expect("HS_FIRMWARE")).unwrap();
    let source = std::fs::read(std::env::var("HS_EEPROM").expect("HS_EEPROM")).unwrap();
    let a_image = image(&source, 0x21);
    let b_image = image(&source, 0x42);
    let mut a = Machine::new(Images {
        firmware: &firmware,
        eeprom: &a_image,
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap();
    let mut b = Machine::new(Images {
        firmware: &firmware,
        eeprom: &b_image,
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap();
    let mut ab = Channel {
        incoming: VecDeque::new(),
        transitions: 0,
    };
    let mut ba = Channel {
        incoming: VecDeque::new(),
        transitions: 0,
    };
    let mut a_buttons = buttons(0);
    let mut b_buttons = buttons(50_000);
    a.run_until(Time::from_micros(4_400_000), &[], &mut ab)
        .unwrap();
    b.run_until(Time::from_micros(4_400_000), &[], &mut ba)
        .unwrap();
    // A selected one-microsecond channel delay bounds both machines' progress.
    // It preserves pulse lengths and needs no knowledge of packet boundaries.
    for micros in 4_400_001..=12_000_000 {
        let end = Time::from_micros(micros);
        advance(&mut a, end, &mut a_buttons, &mut ba.incoming, &mut ab);
        advance(&mut b, end, &mut b_buttons, &mut ab.incoming, &mut ba);
    }
    eprintln!(
        "optical transitions: {} / {}",
        ab.transitions, ba.transitions
    );
    // pw's PeerFinalize shifts the received record into history slot 1.
    // Slot 0 alone would establish only a partial transfer.
    let start = RECORDS + RECORD_SIZE + ID_OFFSET;
    assert_eq!(&a.eeprom()[start..start + 40], &[0x42; 40]);
    assert_eq!(&b.eeprom()[start..start + 40], &[0x21; 40]);
}
