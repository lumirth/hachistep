//! NT7508 serial command parser and controller RAM. The visible 96x64 view is
//! derived; it is not separate authoritative storage. A chip-select boundary
//! clears a partial serial byte but does not erase a pending command parameter.
use crate::{error::Error, signals::{Event, Output}, time::Time};
pub const LCD_WIDTH: usize = 96;
pub const LCD_HEIGHT: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nt7508 {
    ram: [u8; 4096],
    icons: [u8; 256],
    selected: bool, data: bool, shift: u8, bits: u8,
    pending: Option<u8>,
    page: u8, column_byte: u16,
    start_line: u8, icon: bool,
    display_on: bool, entire_on: bool, reverse: bool,
    segment_reverse: bool, common_reverse: bool,
    power_save: bool, contrast: u8,
    saved_column: u16,
    controls: [u8; 24],
}
impl Default for Nt7508 { fn default() -> Self { Self::new() } }
impl Nt7508 {
    pub fn new() -> Self {
        Self { ram: [0; 4096], icons: [0; 256], selected: false, data: false, shift: 0, bits: 0,
            pending: None, page: 0, column_byte: 0, start_line: 0, icon: false,
            display_on: false, entire_on: false, reverse: false, segment_reverse: false,
            common_reverse: false, power_save: false, contrast: 0x20,
            saved_column: 0, controls: [0, 127, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0] }
    }
    pub fn select(&mut self, selected: bool) {
        if self.selected != selected { self.shift = 0; self.bits = 0; }
        self.selected = selected;
    }
    pub fn command_data(&mut self, data: bool) { self.data = data; }
    pub fn rising(&mut self, mosi: bool, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        if !self.selected { return Ok(()); }
        self.shift = self.shift << 1 | u8::from(mosi); self.bits += 1;
        if self.bits == 8 {
            let v = self.shift; self.bits = 0; self.shift = 0;
            if self.data { self.write_data(v, now, output); } else { self.command(v, now, output)?; }
        }
        Ok(())
    }
    pub fn ram(&self) -> &[u8; 4096] { &self.ram }
    pub fn contrast(&self) -> u8 { self.contrast }
    pub fn start_line(&self) -> u8 { self.start_line }
    pub fn enabled(&self) -> bool { self.display_on && !self.power_save }
    pub fn write_counted_fixture(&mut self, data: bool, byte: u8, now: Time, out: &mut dyn Output) -> Result<(), Error> {
        self.select(true); self.command_data(data);
        for n in (0..8).rev() { self.rising(byte & (1 << n) != 0, now, out)?; }
        self.select(false); Ok(())
    }
    fn write_data(&mut self, byte: u8, now: Time, output: &mut dyn Output) {
        let col = usize::from(self.column_byte & 255);
        if self.icon { self.icons[col] = byte; }
        else { self.ram[usize::from(self.page & 15) * 256 + col] = byte; }
        output.event(Event::LcdWrite { at: now, page: if self.icon { 16 } else { self.page }, column_byte: self.column_byte, value: byte });
        // Nominal wrap witness. Terminal-column wrap/lock ambiguity remains
        // explicitly identified in STATUS.md; it is not hardware-certified.
        self.column_byte = (self.column_byte + 1) & 255;
    }
    fn command(&mut self, byte: u8, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        if let Some(command) = self.pending.take() {
            match command {
                0x40..=0x43 => self.start_line = byte & 127,
                0x44..=0x47 => self.controls[0] = byte & 127,
                0x48..=0x4b => self.controls[1] = byte,
                0x4c..=0x4f => self.controls[2] = byte & 31,
                0x81 => self.contrast = byte & 0x3f,
                0x88..=0x8f => self.controls[8 + usize::from(command - 0x88)] = byte,
                0xe8 => self.controls[16] = byte,
                0xf1 => self.controls[17] = byte & 1,
                0xf7 => self.controls[18] = byte & 3,
                0xf6 => self.controls[19] = byte & 31,
                0xf3 => self.controls[20] = byte & 31,
                0xf4 => self.controls[21] = byte & 3,
                _ => return Err(Error::Internal("unrecognized pending LCD parameter")),
            }
            output.event(Event::LcdControl { at: now, command, parameter: Some(byte) });
            return Ok(());
        }
        match byte {
            0x00..=0x0f => self.column_byte = ((self.column_byte >> 1 & 0x70) | u16::from(byte)) << 1,
            0x10..=0x17 => self.column_byte = ((self.column_byte >> 1 & 0x0f) | u16::from(byte & 7) << 4) << 1,
            0x40..=0x4f | 0x81 | 0x88..=0x8f | 0xe8 | 0xf1 | 0xf3 | 0xf4 | 0xf6 | 0xf7 => { self.pending = Some(byte); return Ok(()); }
            0xb0..=0xbf => { self.page = byte & 15; self.icon = false; }
            0xa0 => self.segment_reverse = false,
            0xa1 => self.segment_reverse = true,
            0xa2 => self.icon = false,
            0xa3 => self.icon = true,
            0xa4 => self.entire_on = false,
            0xa5 => self.entire_on = true,
            0xa6 => self.reverse = false,
            0xa7 => self.reverse = true,
            0xa8 => self.power_save = false,
            0xab => self.controls[22] = 1,
            0xa9 => self.power_save = true,
            0xae => self.display_on = false,
            0xaf => self.display_on = true,
            0xc0..=0xc7 => self.common_reverse = false,
            0xc8..=0xcf => self.common_reverse = true,
            0xe0 => self.saved_column = self.column_byte,
            0xe1 => self.power_save = false,
            0xe2 => {
                self.page = 0; self.column_byte = 0; self.start_line = 0;
                self.saved_column = 0; self.controls[3] = 0; self.contrast = 32;
                self.controls[16] = 0; self.controls[7] = 0;
                self.controls[8..12].fill(0); self.controls[12..16].fill(255);
                self.pending = None;
            }
            0xee => self.column_byte = self.saved_column,
            0xe3 => {},
            0xe4 => self.controls[2] = 0,
            // Retained, non-raster drive controls. Their analog panel effects
            // are deliberately not claimed to be implemented.
            0x20..=0x27 => self.controls[3] = byte & 7,
            0x28..=0x2f => self.controls[4] = byte & 7,
            0x64..=0x67 | 0x6c..=0x6f => self.controls[5] = byte & 11,
            0x50..=0x57 => self.controls[6] = byte & 7,
            0x90..=0x97 => self.controls[7] = byte & 7,
            _ => return Err(Error::Unsupported { component: "NT7508", detail: "command not implemented; no silent NOP substitution", address: u16::from(byte) }),
        }
        output.event(Event::LcdControl { at: now, command: byte, parameter: None });
        Ok(())
    }
    /// Logical shade codes. This does not claim analog panel luminance or scan
    /// waveform fidelity; the raw controller RAM remains available to fixtures.
    pub fn render(&self, pixels: &mut [u8; LCD_WIDTH * LCD_HEIGHT]) {
        for y in 0..LCD_HEIGHT { for x in 0..LCD_WIDTH {
            let mut shade = 0;
            if self.enabled() {
                if self.entire_on { shade = 3; }
                else {
                    let logical_y = if self.common_reverse { LCD_HEIGHT - 1 - y } else { y };
                    let row = (logical_y + usize::from(self.start_line) + 128 - usize::from(self.controls[0])) & 127;
                    let col = if self.segment_reverse { LCD_WIDTH - 1 - x } else { x };
                    let index = (row / 8) * 256 + col * 2;
                    shade = ((self.ram[index] >> (row & 7)) & 1) | (((self.ram[index + 1] >> (row & 7)) & 1) << 1);
                }
                if self.reverse { shade ^= 3; }
            }
            pixels[y * LCD_WIDTH + x] = shade;
        }}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameter_survives_chip_select() {
        let mut l = Nt7508::new();
        l.write_counted_fixture(false, 0x81, Time::ZERO, &mut ()).unwrap();
        l.write_counted_fixture(false, 0x35, Time::ZERO, &mut ()).unwrap();
        assert_eq!(l.contrast(), 0x35);
    }
    #[test]
    fn complete_bitplanes_and_nonvisible_ram_survive_reset() {
        let mut l = Nt7508::new();
        for c in [0xb8, 0x10, 0] { l.write_counted_fixture(false, c, Time::ZERO, &mut ()).unwrap(); }
        l.write_counted_fixture(true, 1, Time::ZERO, &mut ()).unwrap();
        l.write_counted_fixture(true, 1, Time::ZERO, &mut ()).unwrap();
        for c in [0x40, 64, 0xaf] { l.write_counted_fixture(false, c, Time::ZERO, &mut ()).unwrap(); }
        let mut pixels = [0; LCD_WIDTH * LCD_HEIGHT]; l.render(&mut pixels); assert_eq!(pixels[0], 3);
        l.write_counted_fixture(false, 0xe2, Time::ZERO, &mut ()).unwrap(); assert_eq!(l.ram()[2048], 1);
    }
}
