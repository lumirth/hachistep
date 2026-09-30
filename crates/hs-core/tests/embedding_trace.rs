use hs_core::{Event, Images, Machine, Output, Time};
use std::ops::ControlFlow;

fn machine() -> Machine {
    let code = [
        0x79, 0, 0x12, 0x34, 0x6b, 0x80, 0xf7, 0x80, // Native RAM word write.
        0x6b, 0, 0xf7, 0x80, // Native RAM word read.
        0xf8, 12, 0x6a, 0x88, 0xff, 0xeb, // P82/P83 output directions.
        0xf8, 4, 0x6a, 0x88, 0xff, 0xdb, // Opposite buzzer terminal levels.
        0x79, 0, 0x12, 0x34, 0x6b, 0x80, 0xff, 0xda, // Two byte-wide SFR lanes.
        0x40, 0xfe,
    ];
    let mut firmware = vec![0; 49_152];
    firmware[..2].copy_from_slice(&[1, 0]);
    firmware[0x100..0x100 + code.len()].copy_from_slice(&code);
    Machine::new(Images {
        firmware: &firmware,
        eeprom: &[255; 65_536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap()
}

// An external Rust caller can exhaustively handle product events without
// forwarding another dependency's choice to enable diagnostic tracing.
#[derive(Default)]
struct Consumer {
    events: Vec<Event>,
    signals: usize,
    persistence: usize,
}
impl Output for Consumer {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        match event {
            Event::Power { .. }
            | Event::LcdWrite { .. }
            | Event::LcdControl { .. }
            | Event::Buzzer { .. }
            | Event::Infrared { .. }
            | Event::Reset { .. } => self.signals += 1,
            Event::NvByte { .. } | Event::NvCommit { .. } | Event::NvInterrupted { .. } => {
                self.persistence += 1
            }
        }
        self.events.push(event);
        ControlFlow::Continue(())
    }
}

#[test]
fn ordinary_embedding_keeps_product_events_separate_from_bus_accesses() {
    let mut machine = machine();
    let mut consumer = Consumer::default();
    let result = machine
        .run_until(Time::from_micros(100), &[], &mut consumer)
        .unwrap();
    assert_eq!(result.now, Time::from_micros(100));
    assert_eq!(&machine.ram()[..2], &[0x12, 0x34]);
    assert!(consumer
        .events
        .iter()
        .any(|e| matches!(e, Event::Buzzer { .. })));
    assert_eq!(consumer.persistence, 0);
    assert_eq!(consumer.signals, consumer.events.len());
    assert!(machine.statistics().bus_reads > consumer.events.len() as u64);
}

#[cfg(feature = "trace")]
mod tracing {
    use super::*;
    use hs_core::{diagnostic, Buttons, Input, Snapshot, StopReason, TimedInput};

    #[test]
    fn opt_in_records_physical_lanes_without_polluting_product_history() {
        let end = Time::from_micros(1000);
        let (mut ordinary, mut traced) = (machine(), machine());
        let (mut a, mut b, mut bus) = (Consumer::default(), Consumer::default(), Vec::new());
        let plain = ordinary.run_until(end, &[], &mut a).unwrap();
        let observed =
            diagnostic::run_until_traced(&mut traced, end, &[], &mut b, &mut bus).unwrap();
        assert_eq!(plain, observed);
        assert_eq!(a.events, b.events);
        assert_eq!(
            ordinary.snapshot().encode().unwrap(),
            traced.snapshot().encode().unwrap()
        );
        assert!(bus
            .iter()
            .any(|e| e.write && e.address == 0xf780 && e.width == 2 && e.value == 0x1234));
        assert!(bus
            .iter()
            .any(|e| !e.write && e.address == 0xf780 && e.width == 2 && e.value == 0x1234));
        let lanes: Vec<_> = bus
            .iter()
            .filter(|e| e.write && (0xffda..=0xffdb).contains(&e.address))
            .collect();
        assert!(lanes.windows(2).any(|pair| {
            pair[0].address == 0xffda
                && pair[0].width == 1
                && pair[0].value == 0x12
                && pair[1].address == 0xffdb
                && pair[1].width == 1
                && pair[1].value == 0x34
                && pair[0].at < pair[1].at
        }));
        assert!(bus.windows(2).all(|pair| pair[0].at <= pair[1].at));

        let (mut x, mut y) = (Vec::new(), Vec::new());
        let (mut audio_a, mut audio_b) = (
            machine().audio(48_000).unwrap(),
            machine().audio(48_000).unwrap(),
        );
        for event in a.events {
            audio_a
                .event(event, &mut |s| x.extend_from_slice(s))
                .unwrap();
        }
        for event in b.events {
            audio_b
                .event(event, &mut |s| y.extend_from_slice(s))
                .unwrap();
        }
        audio_a
            .advance(end, &mut |s| x.extend_from_slice(s))
            .unwrap();
        audio_b
            .advance(end, &mut |s| y.extend_from_slice(s))
            .unwrap();
        assert_eq!(x, y);
        assert!(x.iter().any(|sample| *sample != 0));
        #[cfg(feature = "profile-work")]
        assert!(
            diagnostic::work(&ordinary).interval_exits.horizon > 0,
            "compiling trace without an observer must retain ordinary CPU intervals"
        );
    }

    #[test]
    fn bus_observer_failure_finishes_the_instant_and_resumes_without_repeating_access() {
        let end = Time::from_micros(100);
        let mut reference = machine();
        let mut expected_bus = Vec::new();
        diagnostic::run_until_traced(&mut reference, end, &[], &mut (), &mut expected_bus).unwrap();
        let at = expected_bus
            .iter()
            .find(|e| e.write && e.address == 0xffdb)
            .unwrap()
            .at;
        let inputs = [
            TimedInput {
                at,
                input: Input::TemperatureMillicelsius(27_000),
            },
            TimedInput {
                at,
                input: Input::Buttons(Buttons {
                    center: true,
                    ..Buttons::RELEASED
                }),
            },
            TimedInput {
                at: Time::from_raw(at.raw() + 1),
                input: Input::Power(false),
            },
        ];
        let mut whole = machine();
        let (mut expected, mut expected_bus) = (Vec::new(), Vec::new());
        diagnostic::run_until_traced(&mut whole, end, &inputs, &mut expected, &mut expected_bus)
            .unwrap();
        let mut split = machine();
        let (mut product, mut bus, mut host_error) = (Vec::new(), Vec::new(), None);
        let result = diagnostic::run_until_traced(
            &mut split,
            end,
            &inputs,
            &mut product,
            &mut |event: diagnostic::BusEvent| {
                bus.push(event);
                if event.write && event.address == 0xffdb {
                    host_error = Some(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        )
        .unwrap();
        assert_eq!(result.reason, StopReason::Output);
        assert_eq!(result.now.raw(), at.raw() + 1);
        assert_eq!(result.inputs_consumed, 2);
        assert_eq!(host_error.unwrap().kind(), std::io::ErrorKind::BrokenPipe);
        assert!(split.fault().is_none());
        assert!(product
            .iter()
            .any(|e| matches!(e, Event::Buzzer { at: t, .. } if *t == at)));
        let bytes = split.snapshot().encode().unwrap();
        split = Machine::from_snapshot(&Snapshot::decode(&bytes).unwrap());
        let mut repeated = Vec::new();
        diagnostic::run_until_traced(&mut split, result.now, &[], &mut (), &mut repeated).unwrap();
        assert!(repeated.is_empty());
        diagnostic::run_until_traced(
            &mut split,
            end,
            &inputs[result.inputs_consumed..],
            &mut product,
            &mut bus,
        )
        .unwrap();
        assert_eq!(product, expected);
        assert_eq!(bus, expected_bus);
        assert_eq!(
            split.snapshot().encode().unwrap(),
            whole.snapshot().encode().unwrap()
        );
    }
}
