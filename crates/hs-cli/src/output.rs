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
fn escape(s: &str) -> String {
    let mut r = String::new();
    for c in s.chars() {
        match c {
            '"' => r.push_str("\\\""),
            '\\' => r.push_str("\\\\"),
            '\n' => r.push_str("\\n"),
            '\r' => r.push_str("\\r"),
            '\t' => r.push_str("\\t"),
            c if c.is_control() => r.push_str(&format!("\\u{:04x}", c as u32)),
            _ => r.push(c),
        }
    }
    r
}
pub struct RunMetadata<'a> {
    pub firmware: &'a str,
    pub initial_eeprom: &'a str,
    pub initial_sensor: &'a str,
    pub initial_eeprom_status: u8,
    pub input_hash: Option<&'a str>,
    pub chunk_us: u64,
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
    let conditions = metadata.conditions;
    let input_hash = metadata
        .input_hash
        .map(|v| format!("\"{}\"", escape(v)))
        .unwrap_or_else(|| "null".into());
    let (serial_tx, serial_rx) = m.ssu_counts();
    let fault = failure
        .map(|v| format!("\"{}\"", escape(v)))
        .unwrap_or("null".into());
    let registers = m
        .registers()
        .er
        .iter()
        .map(|v| format!("{v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(concat!("{{\n  \"schema\": 2,\n  \"model\": \"hachistep-development-0.2\",\n",
      "  \"firmware_sha256\": \"{}\",\n  \"initial_eeprom_sha256\": \"{}\",\n",
      "  \"initial_sensor_nv_sha256\": \"{}\",\n  \"initial_eeprom_status\": {},\n  \"input_sha256\": {},\n  \"chunk_us\": {},\n",
      "  \"initial_conditions\": {{\"supply_millivolts\": {}, \"adc_reference_millivolts\": {}, \"main_hz\": {}, \"watch_hz\": {}, \"on_chip_hz\": {}}},\n",
      "  \"requested_time_raw\": \"{}\",\n  \"time_raw\": \"{}\",\n  \"time_us\": {},\n  \"wall_seconds\": {:.9},\n",
      "  \"pc\": {},\n  \"instruction_pc\": {},\n  \"phase\": \"{}\",\n  \"er\": [{}],\n  \"ccr\": {},\n",
      "  \"retired\": {},\n  \"interrupt_entries\": {},\n  \"sleeping\": {},\n  \"display_on\": {},\n  \"display_start\": {},\n",
      "  \"events\": {},\n  \"lcd_events\": {},\n  \"nv_commits\": {},\n  \"buzzer_events\": {},\n  \"ir_events\": {},\n",
      "  \"serial_tx\": {},\n  \"serial_rx\": {},\n  \"bus_reads\": {},\n  \"bus_writes\": {},\n  \"resets\": {},\n",
      "  \"ram_sha256\": \"{}\",\n  \"lcd_ram_sha256\": \"{}\",\n  \"eeprom_sha256\": \"{}\",\n  \"eeprom_status\": {},\n",
      "  \"fault\": {},\n  \"trace_records\": {},\n  \"trace_limit\": {},\n  \"trace_dropped\": {},\n  \"trace_complete\": {}\n}}\n"),metadata.firmware,metadata.initial_eeprom,
      metadata.initial_sensor, metadata.initial_eeprom_status, input_hash, metadata.chunk_us,
      conditions.supply_millivolts, conditions.adc_reference_millivolts,
      conditions.clocks.main_hz, conditions.clocks.watch_hz, conditions.clocks.on_chip_hz,
      metadata.requested.raw(),m.now().raw(),m.now().as_micros(),wall,
      m.registers().pc,m.instruction_pc(),m.phase_name(),registers,m.registers().ccr,m.retired(),m.interrupt_entries(),m.sleeping(),m.display_enabled(),m.display_start_line(),
      e.count,e.lcd,e.nv,e.buzzer,e.ir,serial_tx,serial_rx,s.bus_reads,s.bus_writes,s.resets,sha256(m.ram()),sha256(m.lcd_ram()),sha256(m.eeprom()),m.eeprom_status(),fault,e.trace_count,e.trace_limit,e.trace_dropped,e.trace.is_some() && e.trace_dropped == 0)
}
pub fn export(m: &Machine, dir: &Path, report: &str) -> io::Result<()> {
    // Caller creates a new directory before running. Each file is create_new;
    // no existing firmware, EEPROM, status sidecar, or user output is replaced.
    for (name, data) in [
        ("eeprom.bin", m.eeprom().as_slice()),
        ("eeprom.status", &[m.eeprom_status()][..]),
        ("ram.bin", m.ram().as_slice()),
        ("lcd-ram.bin", m.lcd_ram().as_slice()),
        ("sensor-nv.bin", m.sensor_nonvolatile().as_slice()),
    ] {
        write_new(&dir.join(name), data)?;
    }
    write_new(&dir.join("frame.pgm"), &frame(m))?;
    write_new(&dir.join("report.json"), report.as_bytes())
}
