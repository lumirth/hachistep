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
    let mut cursor = 0;
    let mut pixels = [0; 6144];
    for frame in 1..=300u128 {
        // The frontend chooses when to present. This cadence leaves the device's
        // independent clocks running at their configured frequencies.
        let end = Time::from_raw((frame << 64) / 60);
        let count = inputs[cursor..].partition_point(|input| input.at < end);
        let result = machine.run_until(end, &inputs[cursor..cursor + count], &mut playback)?;
        cursor += result.inputs_consumed;
        if let Some(error) = playback.error.take() {
            return Err(error.into());
        }
        playback.audio.advance(result.now, &mut |block| {
            playback.samples += block.len();
        })?;
        machine.display(&mut pixels);
        // A frontend presents these pixels and paces calls against its host clock.
    }
    println!(
        "time={:?}; retired={}; frames=300; contrast={}; audio samples={}",
        machine.now(),
        machine.retired(),
        machine.display_contrast(),
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
