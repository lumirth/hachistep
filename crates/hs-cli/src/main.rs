//! Dependency-free deterministic frontend. Input files are never overwritten.
mod digest;
mod output;
mod timeline;
use hs_core::{Conditions, Duration, Images, Machine, Time};
use std::{env, fs, io, path::PathBuf, process::ExitCode, time::Instant};
const HELP:&str="HachiStep runnable development core\n\n  hachistep inspect --firmware FILE --eeprom FILE\n  hachistep run --firmware FILE --eeprom FILE [OPTIONS]\n\nOptions:\n  --milliseconds N   Exclusive emulated horizon (default 1000)\n  --input FILE       Physical input CSV; see docs/INPUTS.md\n  --out DIRECTORY    NEW directory: frame, persistent images, report\n  --frame FILE       NEW binary PGM screenshot\n  --trace FILE       NEW product event trace\n  --bus-trace        Include bus events (build with --features trace)\n  --trace-limit N    Bound trace records (default 100000)\n  --sensor-nv FILE   Optional 19-byte sensor nonvolatile image\n  --status BYTE      EEPROM status, decimal or 0x hex (default 0)\n  --supply-mv N      Physical supply witness (default 3000)\n  --chunk-us N       Host-call partition (default 1000)\n  --peek HEX         Side-effect-free final register inspection; repeatable\n\nThis is a runnable starter, not a completed silicon-accurate emulator.\nSee docs/STATUS.md. All outputs are separate from input files.\n";
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        print!("{HELP}");
        return Ok(());
    }
    if !matches!(command.as_str(), "run" | "inspect") {
        return Err(format!("unknown command {command}").into());
    }
    let (mut firmware, mut eeprom, mut frame, mut trace, mut inputs, mut out_dir) =
        (None, None, None, None, None, None);
    let (mut milliseconds, mut chunk, mut trace_limit) = (1000u64, 1000u64, 100000u64);
    let mut sensor_nv = None;
    let mut status = 0u8;
    let mut conditions = Conditions::default();
    let mut peeks = Vec::new();
    let mut bus_trace = false;
    while let Some(key) = args.next() {
        if key == "--bus-trace" {
            bus_trace = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        match key.as_str() {
            "--firmware" => firmware = Some(PathBuf::from(value)),
            "--eeprom" => eeprom = Some(PathBuf::from(value)),
            "--sensor-nv" => sensor_nv = Some(PathBuf::from(value)),
            "--milliseconds" => milliseconds = value.parse()?,
            "--frame" => frame = Some(PathBuf::from(value)),
            "--input" => inputs = Some(PathBuf::from(value)),
            "--out" => out_dir = Some(PathBuf::from(value)),
            "--trace" => trace = Some(PathBuf::from(value)),
            "--trace-limit" => trace_limit = value.parse()?,
            "--status" => {
                status = if let Some(v) = value.strip_prefix("0x") {
                    u8::from_str_radix(v, 16)?
                } else {
                    value.parse()?
                }
            }
            "--supply-mv" => conditions.supply_millivolts = value.parse()?,
            "--chunk-us" => chunk = value.parse()?,
            "--peek" => peeks.push(u16::from_str_radix(value.trim_start_matches("0x"), 16)?),
            _ => return Err(format!("unknown argument {key}").into()),
        }
    }
    if chunk == 0 {
        return Err("--chunk-us must be positive".into());
    }
    if bus_trace && !cfg!(feature = "trace") {
        return Err("--bus-trace requires cargo build -p hs-cli --features trace".into());
    }
    let firmware = fs::read(firmware.ok_or("--firmware is required")?)?;
    let eeprom = fs::read(eeprom.ok_or("--eeprom is required")?)?;
    let firmware_hash = digest::sha256(&firmware);
    let eeprom_hash = digest::sha256(&eeprom);
    let sensor_nv = sensor_nv.map(fs::read).transpose()?;
    let mut m = Machine::with_persistent_state(
        Images {
            firmware: &firmware,
            eeprom: &eeprom,
            eeprom_status: status,
        },
        conditions,
        sensor_nv.as_deref(),
    )?;
    let sensor_hash = digest::sha256(m.sensor_nonvolatile());
    println!(
        "firmware_bytes={} eeprom_bytes={} reset_pc={:04x}",
        firmware.len(),
        eeprom.len(),
        m.registers().pc
    );
    println!("firmware_sha256={firmware_hash}\neeprom_sha256={eeprom_hash}");
    if command == "inspect" {
        return Ok(());
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
        bus_trace,
        ..Default::default()
    };
    let end = Time::ZERO
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or("time overflow")?;
    let wall = Instant::now();
    let mut failure = None;
    let mut cursor = 0;
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
        match m.run_until(to, &changes[cursor..stop], &mut events) {
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
            requested: end,
            conditions,
        },
        failure_message.as_deref(),
    );
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
