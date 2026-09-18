//! Minimal host integration. No source input or host save file is modified.
use hs_core::{Audio, Buttons, Error, Event, Images, Input, Machine, Output, Time, TimedInput};
use std::ops::ControlFlow;

struct Playback {
    audio: Audio,
    samples: usize,
    error: Option<Error>,
}
impl Output for Playback {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        if self.error.is_none() {
            // A frontend would copy these borrowed blocks to its playback queue.
            self.error = self
                .audio
                .event(event, &mut |block| self.samples += block.len())
                .err();
        }
        if self.error.is_some() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: replay FIRMWARE EEPROM".into());
    }
    let firmware = std::fs::read(&args[0])?;
    let eeprom = std::fs::read(&args[1])?;
    let mut machine = Machine::new(Images {
        firmware: &firmware,
        eeprom: &eeprom,
        eeprom_status: 0,
    })?;
    let inputs = [
        TimedInput {
            at: Time::from_micros(4_500_000),
            input: Input::Buttons(Buttons {
                left: false,
                center: true,
                right: false,
            }),
        },
        TimedInput {
            at: Time::from_micros(4_750_000),
            input: Input::Buttons(Buttons::RELEASED),
        },
    ];
    let mut playback = Playback {
        audio: machine.audio(48_000)?,
        samples: 0,
        error: None,
    };
    machine.run_until(Time::from_micros(5_000_000), &inputs, &mut playback)?;
    if let Some(error) = playback.error {
        return Err(error.into());
    }
    playback
        .audio
        .advance(machine.now(), &mut |block| playback.samples += block.len())?;
    println!(
        "time={:?}; retired={}; audio samples={}",
        machine.now(),
        machine.retired(),
        playback.samples
    );
    let saved = machine.snapshot();
    let bytes = saved.encode()?;
    let saved = hs_core::Snapshot::decode(&bytes)?;
    let mut restored = Machine::from_snapshot(&saved);
    let mut original_future = Vec::new();
    let mut restored_future = Vec::new();
    machine.run_until(Time::from_micros(6_000_000), &[], &mut original_future)?;
    restored.run_until(Time::from_micros(6_000_000), &[], &mut restored_future)?;
    assert_eq!(original_future, restored_future);
    assert!(machine.snapshot().encode()? == restored.snapshot().encode()?);
    println!("Restored execution and complete product events agree.");
    Ok(())
}
