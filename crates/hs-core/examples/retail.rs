//! Complete retail sessions using the matching firmware and a save with a walking Pokemon.
#[path = "retail/link.rs"]
mod link;
#[path = "retail/play.rs"]
mod play;
#[path = "retail/state.rs"]
mod state;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: retail play|link FIRMWARE EEPROM".into());
    }
    let firmware = std::fs::read(&args[1])?;
    let eeprom = std::fs::read(&args[2])?;
    match args[0].to_str() {
        Some("play") => play::run(&firmware, &eeprom),
        Some("link") => link::run(&firmware, &eeprom),
        _ => return Err("choose play or link".into()),
    }
    Ok(())
}
