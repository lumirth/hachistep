use crate::digest::sha256;
use hs_core::{Event, Machine, Output, Time};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Default)]
pub struct Events {
    pub count: u64,
    pub lcd: u64,
    pub nv: u64,
    pub buzzer: u64,
    pub ir: u64,
    pub trace: Option<io::BufWriter<fs::File>>,
    pub trace_count: u64,
    pub trace_dropped: u64,
    pub trace_limit: u64,
    pub error: Option<io::Error>,
    pub bus_trace: bool,
}
impl Output for Events {
    fn event(&mut self, event: Event) {
        let is_bus = match event {
            #[cfg(feature = "trace")]
            Event::Bus { .. } => true,
            _ => false,
        };
        if !is_bus {
            self.count += 1;
        }
        match event {
            Event::LcdWrite { .. } | Event::LcdControl { .. } => self.lcd += 1,
            Event::NvCommit { .. } => self.nv += 1,
            Event::Buzzer { .. } => self.buzzer += 1,
            Event::Infrared { .. } => self.ir += 1,
            _ => {}
        }
        if is_bus && !self.bus_trace {
            return;
        }
        if self.trace.is_some() && self.trace_count >= self.trace_limit {
            self.trace_dropped = self.trace_dropped.saturating_add(1);
        }
        if self.error.is_none() && self.trace_count < self.trace_limit {
            if let Some(f) = &mut self.trace {
                if let Err(e) = writeln!(f, "{:032x}\t{event:?}", event.time().raw()) {
                    self.error = Some(e);
                }
                self.trace_count += 1;
            }
        }
    }
}
impl Events {
    pub fn check(&mut self) -> io::Result<()> {
        if let Some(e) = self.error.take() {
            Err(e)
        } else {
            Ok(())
        }
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.check()?;
        if let Some(f) = &mut self.trace {
            f.flush()?;
            f.get_ref().sync_all()?;
        }
        Ok(())
    }
}
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}
pub fn frame(m: &Machine) -> Vec<u8> {
    let mut pixels = [0u8; 6144];
    m.display(&mut pixels);
    let mut data = b"P5\n96 64\n255\n".to_vec();
    data.extend(pixels.iter().map(|v| 255 - *v * 85));
    data
}
pub struct RunMetadata<'a> {
    pub firmware: &'a str,
    pub initial_eeprom: &'a str,
    pub initial_sensor: &'a str,
    pub initial_eeprom_status: u8,
    pub input_hash: Option<&'a str>,
    pub chunk_us: u64,
    pub start: Time,
    pub requested: Time,
    pub conditions: hs_core::Conditions,
}
pub fn report(
    m: &Machine,
    e: &Events,
    wall: f64,
    metadata: &RunMetadata<'_>,
    failure: Option<&str>,
) -> String {
    let s = m.statistics();
    let c = metadata.conditions;
    let (serial_tx, serial_rx) = m.ssu_counts();
    let value = serde_json::json!({
        "schema": 2,
        "model": "hachistep-development-0.2",
        "firmware_sha256": metadata.firmware,
        "initial_eeprom_sha256": metadata.initial_eeprom,
        "initial_sensor_nv_sha256": metadata.initial_sensor,
        "initial_eeprom_status": metadata.initial_eeprom_status,
        "input_sha256": metadata.input_hash,
        "chunk_us": metadata.chunk_us,
        "initial_conditions": {
            "supply_millivolts": c.supply_millivolts,
            "temperature_millicelsius": c.temperature_millicelsius,
            "avcc_override_millivolts": c.avcc_override_millivolts,
            "battery_sense_drop_millivolts": c.battery_sense_drop_millivolts,
            "main_hz": c.clocks.main_hz,
            "watch_hz": c.clocks.watch_hz,
            "on_chip_hz": c.clocks.on_chip_hz,
        },
        "start_time_raw": metadata.start.raw().to_string(),
        "requested_time_raw": metadata.requested.raw().to_string(),
        "time_raw": m.now().raw().to_string(),
        "time_us": m.now().as_micros(),
        "wall_seconds": wall,
        "pc": m.registers().pc,
        "instruction_pc": m.instruction_pc(),
        "phase": m.phase_name(),
        "er": m.registers().er,
        "ccr": m.registers().ccr,
        "retired": m.retired(),
        "interrupt_entries": m.interrupt_entries(),
        "sleeping": m.sleeping(),
        "display_on": m.display_enabled(),
        "display_start": m.display_start_line(),
        "events": e.count,
        "lcd_events": e.lcd,
        "nv_commits": e.nv,
        "buzzer_events": e.buzzer,
        "ir_events": e.ir,
        "serial_tx": serial_tx,
        "serial_rx": serial_rx,
        "bus_reads": s.bus_reads,
        "bus_writes": s.bus_writes,
        "resets": s.resets,
        "ram_sha256": sha256(m.ram()),
        "lcd_ram_sha256": sha256(m.lcd_ram()),
        "eeprom_sha256": sha256(&m.eeprom()),
        "eeprom_status": m.eeprom_status(),
        "fault": failure,
        "trace_records": e.trace_count,
        "trace_limit": e.trace_limit,
        "trace_dropped": e.trace_dropped,
        "trace_complete": e.trace.is_some() && e.trace_dropped == 0,
    });
    // Value formatting owns JSON escaping and arbitrary-width integer output.
    format!("{value:#}\n")
}
pub fn export(m: &Machine, dir: &Path, report: &str) -> io::Result<()> {
    // Caller creates a new directory before running. Each file is create_new;
    // no existing firmware, EEPROM, status sidecar, or user output is replaced.
    for (name, data) in [
        ("flash.bin", m.firmware().as_slice()),
        ("eeprom.bin", m.eeprom().as_slice()),
        ("eeprom.status", &[m.eeprom_status()][..]),
        ("ram.bin", m.ram().as_slice()),
        ("lcd-ram.bin", m.lcd_ram().as_slice()),
        ("lcd-icons.bin", m.lcd_icons().as_slice()),
        ("sensor-nv.bin", m.sensor_nonvolatile().as_slice()),
    ] {
        write_new(&dir.join(name), data)?;
    }
    let saved = m.snapshot().encode().map_err(io::Error::other)?;
    write_new(&dir.join("state.bin"), &saved)?;
    write_new(&dir.join("frame.pgm"), &frame(m))?;
    write_new(&dir.join("report.json"), report.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_preserves_times_beyond_u64_microseconds() {
        let mut m = Machine::new(hs_core::Images {
            firmware: &[0; 49152],
            eeprom: &[0xff; 65536],
            eeprom_status: 0,
        })
        .unwrap();
        m.power_off(&mut ()).unwrap();
        m.run_until(Time::MAX, &[], &mut ()).unwrap();
        let text = report(
            &m,
            &Events::default(),
            0.0,
            &RunMetadata {
                firmware: "unused",
                initial_eeprom: "unused",
                initial_sensor: "unused",
                initial_eeprom_status: 0,
                input_hash: None,
                chunk_us: 1,
                start: Time::ZERO,
                requested: Time::MAX,
                conditions: m.conditions(),
            },
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value["time_us"].as_number().and_then(|v| v.as_u128()),
            Some(Time::MAX.as_micros())
        );
        assert_eq!(value["time_raw"], Time::MAX.raw().to_string());
    }
}
