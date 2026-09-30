//! Deterministic frontend. Input files are never overwritten.
#![recursion_limit = "256"] // Expansion depth of the report's JSON object.
mod args;
mod digest;
mod output;
mod timeline;
use hs_core::{Conditions, Duration, Images, Machine, Time};
use std::{fs, io, process::ExitCode, time::Instant};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    use std::io::Read;
    let (options, inspect) = match args::Cli::parse().command {
        args::Command::Inspect(options) => (options, true),
        args::Command::Run(options) => (options, false),
    };
    let args::Options {
        firmware,
        eeprom,
        load_state,
        save_state,
        milliseconds,
        input: inputs,
        out: out_dir,
        frame,
        trace,
        bus_trace,
        trace_limit,
        sensor_nv,
        status,
        supply_mv,
        avcc_mv,
        battery_drop_mv,
        chunk_us: chunk,
        peek: peeks,
    } = options;
    if bus_trace && !cfg!(feature = "trace") {
        return Err("--bus-trace requires cargo build -p hs-cli --features trace".into());
    }
    let mut m = if let Some(path) = load_state {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take((hs_core::Snapshot::MAX_ENCODED_SIZE + 1) as u64)
            .read_to_end(&mut bytes)?;
        Machine::from_snapshot(&hs_core::Snapshot::decode(&bytes)?)
    } else {
        let firmware = fs::read(firmware.ok_or("--firmware is required")?)?;
        let eeprom = fs::read(eeprom.ok_or("--eeprom is required")?)?;
        let sensor_nv = sensor_nv.map(fs::read).transpose()?;
        Machine::with_conditions(
            Images {
                firmware: &firmware,
                eeprom: &eeprom,
                eeprom_status: status,
                sensor_nonvolatile: sensor_nv.as_deref(),
            },
            Conditions {
                supply_millivolts: supply_mv,
                avcc_override_millivolts: avcc_mv,
                battery_sense_drop_millivolts: battery_drop_mv,
                ..Conditions::default()
            },
        )?
    };
    let start = m.now();
    let conditions = m.conditions();
    let status = m.eeprom_status();
    let firmware_hash = m
        .firmware_origin()
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    let eeprom_hash = digest::sha256(&m.eeprom());
    let sensor_hash = digest::sha256(&m.sensor_nonvolatile());
    println!(
        "firmware_sha256={firmware_hash}\neeprom_sha256={eeprom_hash}\ntime={:?}",
        m.now()
    );
    if inspect {
        return Ok(());
    }
    let end = Time::ZERO
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or("time overflow")?;
    if end < m.now() {
        return Err("requested horizon precedes the loaded state".into());
    }
    let timeline_text = inputs.map(fs::read_to_string).transpose()?;
    let input_hash = timeline_text
        .as_ref()
        .map(|text| digest::sha256(text.as_bytes()));
    let changes = timeline_text
        .as_deref()
        .map(timeline::parse)
        .transpose()?
        .unwrap_or_default();
    if let Some(dir) = &out_dir {
        fs::create_dir(dir)?;
    }
    let trace = trace
        .map(|p| {
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(p)
                .map(io::BufWriter::new)
        })
        .transpose()?;
    let mut events = output::Events {
        trace,
        trace_limit,
        ..Default::default()
    };
    let wall = Instant::now();
    let mut failure = None;
    let mut cursor = changes.partition_point(|input| input.at < m.now());
    while m.now() < end {
        let to = m
            .now()
            .checked_add(Duration::from_micros(chunk))
            .ok_or("time overflow")?
            .min(end);
        let mut stop = cursor;
        while stop < changes.len() && changes[stop].at < to {
            stop += 1;
        }
        match events.run_until(&mut m, to, &changes[cursor..stop], bus_trace) {
            Ok(result) => cursor += result.inputs_consumed,
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
        events.check()?;
    }
    let elapsed = wall.elapsed().as_secs_f64();
    events.finish()?;
    let failure_message = failure.as_ref().map(ToString::to_string);
    let report = output::report(
        &m,
        &events,
        elapsed,
        &output::RunMetadata {
            firmware: &firmware_hash,
            initial_eeprom: &eeprom_hash,
            initial_sensor: &sensor_hash,
            initial_eeprom_status: status,
            input_hash: input_hash.as_deref(),
            chunk_us: chunk,
            start,
            requested: end,
            conditions,
        },
        failure_message.as_deref(),
    );
    if let Some(path) = save_state {
        output::write_new(&path, &m.snapshot().encode()?)?;
    }
    if let Some(path) = frame {
        output::write_new(&path, &output::frame(&m))?;
    }
    if let Some(dir) = out_dir {
        output::export(&m, &dir, &report)?;
    }
    println!("time={:?} wall_seconds={elapsed:.6} pc={:04x} instruction_pc={:04x} phase={} retired={} interrupts={} sleeping={} display_on={} display_start={}",
        m.now(),m.registers().pc,m.instruction_pc(),m.phase_name(),m.retired(),m.interrupt_entries(),m.sleeping(),m.display_enabled(),m.display_start_line());
    println!(
        "events={} lcd={} nv_commits={} buzzer={} infrared={} serial={:?} statistics={:?}",
        events.count,
        events.lcd,
        events.nv,
        events.buzzer,
        events.ir,
        m.ssu_counts(),
        m.statistics()
    );
    println!("er={:08x?} ccr={:02x}", m.registers().er, m.registers().ccr);
    for a in peeks {
        println!("peek_{a:04x}={:?}", m.peek(a));
    }
    if let Some(e) = failure {
        return Err(e.into());
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
