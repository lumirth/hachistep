use hs_core::{signals::Piezo, Audio, Event, Time};

fn waveform() -> Vec<Event> {
    // 400 Hz, with an abrupt switch to silence that cuts a negative half-cycle.
    (0..800)
        .map(|i| Event::Buzzer {
            at: Time::from_micros(i * 1250),
            drive: if i % 2 == 0 {
                Piezo::Positive
            } else {
                Piezo::Negative
            },
        })
        .chain([Event::Buzzer {
            at: Time::from_micros(999_500),
            drive: Piezo::Neutral,
        }])
        .collect()
}

#[test]
fn output_rate_pitch_and_silence_follow_emulated_time() {
    for rate in [44_100, 48_000] {
        let mut audio = Audio::new(rate, Time::ZERO, Piezo::Neutral).unwrap();
        let mut pcm = Vec::new();
        for event in waveform() {
            audio
                .event(event, &mut |s| pcm.extend_from_slice(s))
                .unwrap();
        }
        audio
            .advance(Time::from_micros(1_100_000), &mut |s| {
                pcm.extend_from_slice(s)
            })
            .unwrap();
        assert_eq!(pcm.len(), rate as usize * 11 / 10);
        let crossings = pcm[rate as usize / 10..rate as usize / 2]
            .windows(2)
            .filter(|w| w[0] <= 0 && w[1] > 0)
            .count();
        assert!(
            (159..=161).contains(&crossings),
            "{crossings} crossings at {rate} Hz"
        );
        assert!(pcm.iter().any(|s| *s > 4000));
        assert!(pcm.iter().any(|s| *s < -4000));
        assert!(pcm[pcm.len() - 100..].iter().all(|s| s.abs() < 4));
    }
}

#[test]
fn arbitrary_execution_partitions_preserve_every_sample() {
    let mut whole = Audio::new(44_100, Time::ZERO, Piezo::Neutral).unwrap();
    let mut split = Audio::new(44_100, Time::ZERO, Piezo::Neutral).unwrap();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut next = 13;
    for event in waveform() {
        whole.event(event, &mut |s| a.extend_from_slice(s)).unwrap();
        while Time::from_micros(next) < event.time() {
            split
                .advance(Time::from_micros(next), &mut |s| b.extend_from_slice(s))
                .unwrap();
            next += 137;
        }
        split.event(event, &mut |s| b.extend_from_slice(s)).unwrap();
    }
    let end = Time::from_micros(1_010_000);
    whole.advance(end, &mut |s| a.extend_from_slice(s)).unwrap();
    split.advance(end, &mut |s| b.extend_from_slice(s)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn rejected_time_and_rate_do_not_disrupt_a_stream() {
    assert!(Audio::new(0, Time::ZERO, Piezo::Neutral).is_err());
    let origin = Time::from_micros(10_000_000);
    let mut audio = Audio::new(48_000, origin, Piezo::Neutral).unwrap();
    assert!(audio
        .advance(Time::ZERO, &mut |_| panic!("no samples expected"))
        .is_err());
    let mut count = 0;
    audio
        .advance(Time::from_micros(10_001_000), &mut |s| count += s.len())
        .unwrap();
    assert_eq!(count, 48);
}

#[test]
fn a_restored_machine_supplies_the_existing_buzzer_drive() {
    use hs_core::{Images, Machine, Snapshot};
    let mut firmware = vec![0; 49_152];
    firmware[..2].copy_from_slice(&0x100u16.to_be_bytes());
    // Configure P82/P83 as outputs and hold opposite levels.
    let code = [
        0xf8, 12, 0x6a, 0x88, 0xff, 0xeb, 0xf8, 4, 0x6a, 0x88, 0xff, 0xdb, 0x40, 0xfe,
    ];
    firmware[0x100..0x100 + code.len()].copy_from_slice(&code);
    let mut machine = Machine::new(Images {
        firmware: &firmware,
        eeprom: &[255; 65_536],
        eeprom_status: 0,
    })
    .unwrap();
    machine
        .run_until(Time::from_micros(100), &[], &mut ())
        .unwrap();
    let snapshot = machine.snapshot().encode().unwrap();
    let restored = Machine::from_snapshot(&Snapshot::decode(&snapshot).unwrap());
    let mut a = machine.audio(48_000).unwrap();
    let mut b = restored.audio(48_000).unwrap();
    let (mut x, mut y) = (Vec::new(), Vec::new());
    a.advance(Time::from_micros(1100), &mut |s| x.extend_from_slice(s))
        .unwrap();
    b.advance(Time::from_micros(1100), &mut |s| y.extend_from_slice(s))
        .unwrap();
    assert!(x.iter().any(|s| s.abs() > 4000));
    assert_eq!(x, y);
    assert_eq!(machine.snapshot().encode().unwrap(), snapshot);
}
