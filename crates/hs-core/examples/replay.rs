//! Minimal host integration. No source input or host save file is modified.
use hs_core::{Buttons, Images, Input, Machine, Time, TimedInput};
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
    // Vec is an application-chosen output collector. The core never requires one.
    let mut events = Vec::new();
    machine.run_until(Time::from_micros(5_000_000), &inputs, &mut events)?;
    println!(
        "time={:?}; retired={}; events={}",
        machine.now(),
        machine.retired(),
        events.len()
    );
    let saved = machine.snapshot();
    let mut restored = Machine::from_snapshot(&saved);
    let mut original_future = Vec::new();
    let mut restored_future = Vec::new();
    machine.run_until(Time::from_micros(6_000_000), &[], &mut original_future)?;
    restored.run_until(Time::from_micros(6_000_000), &[], &mut restored_future)?;
    assert_eq!(original_future, restored_future);
    assert_eq!(machine, restored);
    println!("Restored execution and complete product events agree.");
    Ok(())
}
