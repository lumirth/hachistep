//! Native adapter for tools/compare_workloads.py and supplied board workload ROMs.
//!
//! Construction and final exports are outside the measured loop. The timing
//! consumer counts product events without allocation; --artifact-dir instead
//! collects a complete untimed history and the canonical native save state.
use hs_core::{Event, Images, Machine, Output, Time};
use std::{fs, io::Write, ops::ControlFlow, path::PathBuf, time::Instant};

#[derive(Default)]
struct Events {
    count: u64,
    hash: u64,
    history: Option<Vec<Event>>,
}
impl Output for Events {
    fn event(&mut self, event: Event) -> ControlFlow<()> {
        self.count += 1;
        // Preserve the supplied legacy adapter's endpoint checksum. It omits
        // event time and several fields; only the exported history is complete.
        let value = match event {
            Event::LcdWrite {
                page,
                column_byte,
                value,
                ..
            } => u64::from(value) | (u64::from(page) << 8) | (u64::from(column_byte) << 16),
            Event::Infrared { emitting, .. } => u64::from(emitting),
            _ => 0,
        };
        self.hash = (self.hash ^ value).wrapping_mul(1_099_511_628_211);
        if let Some(history) = &mut self.history {
            history.push(event);
        }
        ControlFlow::Continue(())
    }
}

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(14_695_981_039_346_656_037, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1_099_511_628_211)
    })
}

#[cfg(feature = "profile-work")]
fn work_json(work: hs_core::diagnostic::Work) -> String {
    let exits = work.interval_exits;
    format!(
        concat!(
            ",\"work\":{{\"cpu_phase_dispatches\":{},\"interval_entries\":{},",
            "\"interval_exits\":{{\"request\":{},\"committed_owner\":{},",
            "\"horizon\":{},\"exception\":{},\"sleep\":{},\"reset\":{},\"error\":{}}},",
            "\"owner_sync_calls\":{},\"owner_syncs\":{},\"board_settlements\":{},",
            "\"board_resolutions\":{},\"serial_resolutions\":{},\"serial_resolution_lanes\":{},",
            "\"serial_edge_deliveries\":{},\"sensor_sample_phases\":{},",
            "\"serial_prefixes\":{},\"serial_prefix_edges\":{},",
            "\"cpu_time_materializations\":{}}}"
        ),
        work.cpu_phase_dispatches,
        work.interval_entries,
        exits.request,
        exits.committed_owner,
        exits.horizon,
        exits.exception,
        exits.sleep,
        exits.reset,
        exits.error,
        work.owner_sync_calls,
        work.owner_syncs,
        work.board_settlements,
        work.board_resolutions,
        work.serial_resolutions,
        work.serial_resolution_lanes,
        work.serial_edge_deliveries,
        work.sensor_sample_phases,
        work.serial_prefixes,
        work.serial_prefix_edges,
        work.cpu_time_materializations,
    )
}

fn profile_replays(
    rom: &[u8],
    mode: &str,
    quantum: u64,
    limit: u64,
    seconds: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Sampling needs a sustained process. Each replay still executes the whole
    // supplied guest; construction is part of this profiling view, not timing.
    println!(
        "{{\"profile_ready\":true,\"instrumented\":{}}}",
        cfg!(feature = "profile-work")
    );
    std::io::stdout().flush()?;
    let start = Instant::now();
    let mut replays = 0;
    while start.elapsed().as_secs() < seconds {
        let mut machine = Machine::new(Images {
            firmware: rom,
            eeprom: &[0xff; 65_536],
            eeprom_status: 0,
            sensor_nonvolatile: None,
        })?;
        let mut events = Events::default();
        let mut horizon = 0;
        while horizon < limit {
            horizon = horizon.saturating_add(quantum).min(limit);
            machine.run_until(Time::from_micros(horizon), &[], &mut events)?;
            if mode == "job" && machine.ram()[0x70] == 0xa5 && machine.sleeping() {
                break;
            }
        }
        std::hint::black_box((&machine, events.count));
        replays += 1;
    }
    println!("{{\"mode\":\"profiling\",\"replays\":{replays},\"profile_seconds\":{seconds}}}");
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 5 || !matches!(args[0].as_str(), "job" | "time") {
        return Err(
            "usage: workload_probe job|time ROM QUANTUM_US LIMIT_US 1 [--artifact-dir DIR | --profile-seconds N]".into(),
        );
    }
    let quantum: u64 = args[2].parse()?;
    let limit: u64 = args[3].parse()?;
    if quantum == 0 || limit == 0 {
        return Err("quantum and limit must be positive".into());
    }
    let (artifacts, profile_seconds) = match &args[5..] {
        [] => (None, None),
        [flag, path] if flag == "--artifact-dir" => (Some(PathBuf::from(path)), None),
        [flag, seconds] if flag == "--profile-seconds" => (None, Some(seconds.parse::<u64>()?)),
        _ => {
            return Err(
                "expected --artifact-dir DIR, --profile-seconds N or no trailing arguments".into(),
            )
        }
    };
    let rom = fs::read(&args[1])?;
    if let Some(seconds) = profile_seconds {
        if seconds == 0 {
            return Err("profile seconds must be positive".into());
        }
        return profile_replays(&rom, &args[0], quantum, limit, seconds);
    }
    let mut machine = Machine::new(Images {
        firmware: &rom,
        eeprom: &[0xff; 65_536],
        eeprom_status: 0,
        sensor_nonvolatile: None,
    })?;
    let mut events = Events {
        history: artifacts.as_ref().map(|_| Vec::new()),
        ..Events::default()
    };
    let mut horizon = 0;
    let mut calls = 0;
    #[cfg(feature = "profile-work")]
    let before = hs_core::diagnostic::work(&machine);
    let start = Instant::now();
    while horizon < limit {
        horizon = horizon.saturating_add(quantum).min(limit);
        machine.run_until(Time::from_micros(horizon), &[], &mut events)?;
        calls += 1;
        if args[0] == "job" && machine.ram()[0x70] == 0xa5 && machine.sleeping() {
            break;
        }
    }
    let ns = start.elapsed().as_nanos();
    #[cfg(feature = "profile-work")]
    let work = work_json(hs_core::diagnostic::work(&machine).since(before));
    #[cfg(not(feature = "profile-work"))]
    let work = "";
    let mut artifact_json = String::new();
    if let Some(directory) = artifacts {
        // The comparison tool creates this directory. Files are new so an old
        // successful history cannot stand in for a failed current export.
        let write_new = |name: &str, bytes: &[u8]| -> std::io::Result<()> {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(name))?
                .write_all(bytes)
        };
        write_new("state.bin", &machine.snapshot().encode()?)?;
        let history = events.history.as_ref().unwrap();
        let mut encoded = String::new();
        for event in history {
            use std::fmt::Write;
            writeln!(&mut encoded, "{:032x}\t{event:?}", event.time().raw())?;
        }
        write_new("events.txt", encoded.as_bytes())?;
        artifact_json = format!(
            ",\"history\":{{\"path\":\"events.txt\",\"complete\":true,\"records\":{}}}",
            history.len()
        );
    }
    let (tx, rx) = machine.ssu_counts();
    println!(
        concat!(
            "{{\"core\":\"hachistep\",\"mode\":\"{}\",\"ns\":{},\"horizon_us\":{},",
            "\"calls\":{},\"retired\":{},\"sleeping\":{},\"completed\":{},",
            "\"pc\":{},\"ccr\":{},\"registers\":{:?},\"ram_hash\":{},",
            "\"lcd_hash\":{},\"eeprom_hash\":{},\"ssu_counts\":[{},{}],",
            "\"events\":{},\"event_hash\":{},\"resets\":{},\"cumulative_counts\":true{}{}}}"
        ),
        args[0],
        ns,
        horizon,
        calls,
        machine.retired(),
        machine.sleeping(),
        machine.ram()[0x70] == 0xa5,
        machine.registers().pc,
        machine.registers().ccr,
        machine.registers().er,
        hash(machine.ram()),
        hash(machine.lcd_ram()),
        hash(&machine.eeprom()),
        tx,
        rx,
        events.count,
        events.hash,
        machine.statistics().resets,
        artifact_json,
        work,
    );
    Ok(())
}
