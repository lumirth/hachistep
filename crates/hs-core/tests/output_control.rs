#[path = "support/state.rs"]
mod state;
use hs_core::{Buttons, Event, Images, Input, Machine, Output, StopReason, Time, TimedInput};
use std::ops::ControlFlow;

fn transmitting() -> Machine {
    let mut code = Vec::new();
    for (address, value) in [
        (0xffd6_u16, 1),
        (0xffe6, 5),
        (0xfffa, 0x43),
        (0xff91, 0xd0),
        (0xff99, 1),
        (0xffa7, 0x80),
        (0xff9a, 0x20),
        (0xff9b, 0xa5),
        (0xffd6, 0),
    ] {
        code.extend([0xf8, value, 0x6a, 0x88, (address >> 8) as u8, address as u8]);
    }
    code.extend([0x40, 0xfe]);
    let mut rom = vec![0; 49_152];
    rom[..2].copy_from_slice(&0x100u16.to_be_bytes());
    rom[0x100..0x100 + code.len()].copy_from_slice(&code);
    Machine::new(Images {
        firmware: &rom,
        eeprom: &[255; 65_536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })
    .unwrap()
}

struct StopOnce {
    matching: fn(Event) -> bool,
    requested_at: Option<Time>,
    events: Vec<Event>,
}
impl StopOnce {
    fn new(matching: fn(Event) -> bool) -> Self {
        Self {
            matching,
            requested_at: None,
            events: Vec::new(),
        }
    }
}
impl Output for StopOnce {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        self.events.push(event);
        if self.requested_at.is_none() && (self.matching)(event) {
            self.requested_at = Some(event.time());
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

#[test]
fn stopping_and_restoring_on_each_signal_preserves_the_complete_session() {
    let end = Time::from_micros(1000);
    let inputs = [
        TimedInput {
            at: Time::from_micros(200),
            input: Input::TemperatureMillicelsius(27_000),
        },
        TimedInput {
            at: Time::from_micros(300),
            input: Input::Buttons(Buttons {
                center: true,
                ..Buttons::RELEASED
            }),
        },
    ];
    let mut whole = transmitting();
    let mut split = whole.clone();
    let mut expected = Vec::new();
    whole.run_until(end, &inputs, &mut expected).unwrap();
    let (mut cursor, mut stops) = (0, 0);
    let mut observed = Vec::new();
    while split.now() < end {
        let mut output = StopOnce::new(|e| matches!(e, Event::Infrared { .. }));
        let before = split.now();
        let result = split
            .run_until(end, &inputs[cursor..], &mut output)
            .unwrap();
        assert!(result.now > before);
        cursor += result.inputs_consumed;
        if let Some(at) = output.requested_at {
            assert_eq!(result.now.raw(), at.raw() + 1);
            stops += 1;
            split = state::restore_file(&split.snapshot());
        }
        observed.extend(output.events);
    }
    assert_eq!(stops, 10);
    assert_eq!(cursor, inputs.len());
    assert_eq!(observed, expected);
    state::assert_same_state(&whole, &split);
}

#[test]
fn a_stop_finishes_coincident_outputs_and_the_entire_input_batch() {
    let end = Time::from_micros(1000);
    let mut machine = transmitting();
    let mut start = StopOnce::new(|e| matches!(e, Event::Infrared { emitting: true, .. }));
    machine.run_until(end, &[], &mut start).unwrap();
    let at = machine.now();
    let after = Time::from_raw(at.raw() + 1);
    let inputs = [
        TimedInput {
            at,
            input: Input::Power(false),
        },
        TimedInput {
            at,
            input: Input::TemperatureMillicelsius(27_000),
        },
        TimedInput {
            at: after,
            input: Input::Power(true),
        },
    ];
    let mut reference = machine.clone();
    let mut expected = Vec::new();
    reference.run_until(after, &inputs, &mut expected).unwrap();
    let mut output = StopOnce::new(|_| true);
    let result = machine.run_until(end, &inputs, &mut output).unwrap();
    assert_eq!(result.now, after);
    assert_eq!(result.inputs_consumed, 2);
    assert_eq!(result.reason, StopReason::Output);
    assert_eq!(machine.conditions().temperature_millicelsius, 27_000);
    assert!(!machine.powered());
    assert!(output.events.contains(&Event::Power { at, on: false }));
    assert!(output.events.contains(&Event::Infrared {
        at,
        emitting: false,
    }));
    assert_eq!(output.events, expected);
    state::assert_same_state(&machine, &reference);

    // The completed instant is past input; the returned horizon accepts input.
    assert!(machine.run_until(end, &inputs, &mut ()).is_err());
    assert!(machine.fault().is_none());
    state::assert_same_state(&machine, &reference);
    let result = machine.run_until(end, &inputs[2..], &mut ()).unwrap();
    assert_eq!(result.inputs_consumed, 1);
    assert_eq!(result.now, end);
    assert_eq!(result.reason, StopReason::Horizon);
    assert!(machine.powered());
}

#[test]
fn an_event_at_the_horizon_remains_pending_until_the_next_call() {
    let mut reference = transmitting();
    let mut output = StopOnce::new(|e| matches!(e, Event::Infrared { .. }));
    reference
        .run_until(Time::from_micros(1000), &[], &mut output)
        .unwrap();
    let at = output.requested_at.unwrap();
    let mut machine = transmitting();
    let mut output = StopOnce::new(|e| matches!(e, Event::Infrared { .. }));
    let result = machine.run_until(at, &[], &mut output).unwrap();
    assert_eq!(result.now, at);
    assert_eq!(result.reason, StopReason::Horizon);
    assert_eq!(output.requested_at, None);
    let after = Time::from_raw(at.raw() + 1);
    let result = machine.run_until(after, &[], &mut output).unwrap();
    assert_eq!(result.now, after);
    assert_eq!(result.reason, StopReason::Output);
    assert_eq!(output.requested_at, Some(at));
    state::assert_same_state(&machine, &reference);
    let mut empty = Vec::new();
    machine.run_until(after, &[], &mut empty).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn a_borrowed_callback_can_return_a_host_failure_without_faulting_the_device() {
    let end = Time::from_micros(1000);
    let mut whole = transmitting();
    let mut split = whole.clone();
    let mut expected = Vec::new();
    whole.run_until(end, &[], &mut expected).unwrap();

    let mut observed = Vec::new();
    let mut host_error = None;
    let result = split
        .run_until(end, &[], &mut |event| {
            observed.push(event);
            if matches!(event, Event::Infrared { .. }) && host_error.is_none() {
                host_error = Some(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .unwrap();
    assert_eq!(result.reason, StopReason::Output);
    assert_eq!(host_error.unwrap().kind(), std::io::ErrorKind::BrokenPipe);
    assert!(split.fault().is_none());

    // Replacing the failed host connection requires no reset or guest repair.
    split = state::restore_file(&split.snapshot());
    let result = split.run_until(end, &[], &mut observed).unwrap();
    assert_eq!(result.reason, StopReason::Horizon);
    assert_eq!(observed, expected);
    state::assert_same_state(&split, &whole);
}
