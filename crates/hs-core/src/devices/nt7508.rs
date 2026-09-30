//! NT7508 serial command parser and controller RAM. The visible 96x64 view
//! derives from that RAM and the controller settings. A chip-select boundary
//! clears a partial serial byte but does not erase a pending command parameter.
use crate::serial::Bits;
use crate::{
    error::Error,
    signals::{Event, Output},
    time::{Duration, Time, TimeError},
};
mod scan;
pub use scan::LcdDrive;
pub const LCD_WIDTH: usize = 96;
pub const LCD_HEIGHT: usize = 64;
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Nt7508 {
    ram: [u8; 4096],
    icons: [u8; 256],
    selected: bool,
    data: bool,
    shift: u8,
    bits: u8,
    pending: Option<u8>,
    page: u8,
    column_byte: u16,
    start_line: u8,
    icon_enabled: bool,
    display_on: bool,
    entire_on: bool,
    reverse: bool,
    segment_reverse: bool,
    common_reverse: bool,
    power_save: bool,
    contrast: u8,
    saved_column: Option<u16>,
    initial_com: u8,
    duty: u8,
    inversion_lines: u8,
    regulator: u8,
    power_control: u8,
    booster: u8,
    bias: u8,
    gray_mode: u8,
    palette: [u8; 8],
    temperature_slope: u8,
    oscillator_control: u8,
    oscillator_frequency: u8,
    contrast_trim: u8,
    otp_control: u8,
    oscillator_enabled: bool,
    scan: scan::Scan,
    supplied: bool,
    analog: bool,
    cold: bool,
    ready: Option<Time>,
}
impl Default for Nt7508 {
    fn default() -> Self {
        Self::new()
    }
}
impl Nt7508 {
    pub(crate) fn serial_effect_edges(&self) -> u8 {
        if self.selected {
            (8 - self.bits) * 2 - 1
        } else {
            16
        }
    }
    pub fn new() -> Self {
        Self {
            ram: [0; 4096],
            icons: [0; 256],
            selected: false,
            data: false,
            shift: 0,
            bits: 0,
            pending: None,
            page: 0,
            column_byte: 0,
            start_line: 0,
            icon_enabled: false,
            display_on: false,
            entire_on: false,
            reverse: false,
            segment_reverse: false,
            common_reverse: false,
            power_save: false,
            contrast: 0x20,
            saved_column: None,
            initial_com: 0,
            duty: 128,
            inversion_lines: 0,
            regulator: 0,
            power_control: 0,
            booster: 0,
            bias: 7,
            gray_mode: 0,
            palette: [0, 0, 0, 0, 255, 255, 255, 255],
            temperature_slope: 0,
            oscillator_control: 0,
            oscillator_frequency: 0,
            contrast_trim: 0,
            otp_control: 0,
            oscillator_enabled: false,
            scan: scan::Scan::default(),
            supplied: true,
            analog: true,
            cold: false,
            ready: None,
        }
    }
    pub(crate) fn set_supply(&mut self, rail: u16, now: Time) -> Result<(), Error> {
        let supplied = rail >= 1650;
        if self.supplied && !supplied {
            self.scan = self.project_scan(now)?;
            self.scan.stop();
            self.select(false);
            self.ready = None;
        }
        if rail == 0 {
            self.cold = true;
        }
        if supplied && !self.supplied {
            if self.cold {
                let (ram, icons) = (self.ram, self.icons);
                *self = Self::new();
                self.ram = ram;
                self.icons = icons;
                self.cold = true;
                self.ready = Some(
                    now.checked_add(Duration::from_micros(21))
                        .ok_or(TimeError::Overflow)?,
                );
            } else {
                self.update_clock(now, true)?;
            }
        }
        self.supplied = supplied;
        self.analog = rail >= 2400;
        Ok(())
    }
    pub(crate) fn lose_volatile(&mut self) {
        self.ram.fill(0);
        self.icons.fill(0);
        self.cold = true;
    }
    pub(crate) fn deadline(&self) -> Option<Time> {
        self.ready
    }
    pub(crate) fn at_deadline(&mut self, now: Time) {
        if self.ready == Some(now) {
            self.ready = None;
            self.cold = false;
        }
    }
    pub fn select(&mut self, selected: bool) {
        if self.selected != selected {
            self.shift = 0;
            self.bits = 0;
        }
        self.selected = selected;
    }
    pub fn command_data(&mut self, data: bool) {
        self.data = data;
    }
    pub fn rising(&mut self, mosi: bool, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        self.receive_bits(Bits::one(mosi), now, output)
    }
    pub(crate) fn receive_bits(
        &mut self,
        bits: Bits,
        now: Time,
        output: &mut dyn Output,
    ) -> Result<(), Error> {
        if !self.selected || !self.supplied || self.ready.is_some() {
            return Ok(());
        }
        bits.append(&mut self.shift, &mut self.bits);
        if self.bits == 8 {
            let v = self.shift;
            self.bits = 0;
            self.shift = 0;
            self.scan = self.project_scan(now)?;
            let was_running = self.oscillator_enabled && !self.power_save;
            if self.data {
                self.write_data(v, now, output);
            } else {
                self.command(v, now, output)?;
            }
            self.update_clock(now, was_running)?;
        }
        Ok(())
    }
    pub fn ram(&self) -> &[u8; 4096] {
        &self.ram
    }
    pub fn icons(&self) -> &[u8; 256] {
        &self.icons
    }
    pub fn contrast(&self) -> u8 {
        self.contrast
    }
    pub fn start_line(&self) -> u8 {
        self.start_line
    }
    pub fn enabled(&self) -> bool {
        self.supplied && self.analog && self.ready.is_none() && self.display_on && !self.power_save
    }
    pub fn write_counted_fixture(
        &mut self,
        data: bool,
        byte: u8,
        now: Time,
        out: &mut dyn Output,
    ) -> Result<(), Error> {
        self.select(true);
        self.command_data(data);
        for n in (0..8).rev() {
            self.rising(byte & (1 << n) != 0, now, out)?;
        }
        self.select(false);
        Ok(())
    }
    fn write_data(&mut self, byte: u8, now: Time, output: &mut dyn Output) {
        let col = usize::from(self.column_byte & 255);
        if self.page == 16 {
            self.icons[col] = byte & 1;
        } else {
            self.ram[usize::from(self.page & 15) * 256 + col] = byte;
        }
        let _ = output.event(Event::LcdWrite {
            at: now,
            page: self.page,
            column_byte: self.column_byte,
            value: byte,
        });
        // The serial-interface and column-command descriptions specify wrap
        // at the 256-byte page boundary; a generic paragraph contradicts them.
        self.column_byte = (self.column_byte + 1) & 255;
    }
    fn command(&mut self, byte: u8, now: Time, output: &mut dyn Output) -> Result<(), Error> {
        if let Some(command) = self.pending.take() {
            match command {
                0x40..=0x43 => self.start_line = byte & 127,
                0x44..=0x47 => self.initial_com = byte & 127,
                0x48..=0x4b if (16..=128).contains(&byte) => self.duty = byte,
                0x48..=0x4b => {}
                0x4c..=0x4f => {
                    // NT7508 p.40: zero selects frame inversion; the other
                    // register values encode intervals from three to 33 lines.
                    let value = byte & 31;
                    self.inversion_lines = if value == 0 { 0 } else { value + 2 };
                }
                0x81 => self.contrast = byte & 0x3f,
                0x88..=0x8f => self.palette[usize::from(command - 0x88)] = byte,
                0xf1 => self.temperature_slope = byte & 1,
                0xf7 => self.oscillator_control = byte & 3,
                0xf6 => self.oscillator_frequency = byte & 31,
                0xf3 => self.contrast_trim = byte & 15,
                0xf4 => self.otp_control = byte & 3,
                _ => return Err(Error::Internal("unrecognized pending LCD parameter")),
            }
            let _ = output.event(Event::LcdControl {
                at: now,
                command,
                parameter: Some(byte),
            });
            return Ok(());
        }
        match byte {
            0x00..=0x0f => {
                self.column_byte = ((self.column_byte >> 1 & 0x70) | u16::from(byte)) << 1
            }
            0x10..=0x17 => {
                self.column_byte = ((self.column_byte >> 1 & 0x0f) | u16::from(byte & 7) << 4) << 1
            }
            0x40..=0x4f | 0x81 | 0x88..=0x8f | 0xf1 | 0xf3 | 0xf4 | 0xf6 | 0xf7 => {
                self.pending = Some(byte);
                return Ok(());
            }
            0xb0..=0xbf => self.page = byte & 15,
            0xa0 => self.segment_reverse = false,
            0xa1 => self.segment_reverse = true,
            0xa2 => self.icon_enabled = false,
            0xa3 => {
                self.icon_enabled = true;
                self.page = 16;
            }
            0xa4 => self.entire_on = false,
            0xa5 => self.entire_on = true,
            0xa6 => self.reverse = false,
            0xa7 => self.reverse = true,
            0xa8 => self.power_save = false,
            0xab => self.oscillator_enabled = true,
            0xa9 => self.power_save = true,
            0xae => self.display_on = false,
            0xaf => self.display_on = true,
            0xc0..=0xc7 => self.common_reverse = false,
            0xc8..=0xcf => self.common_reverse = true,
            0xe0 => self.saved_column = Some(self.column_byte),
            0xe1 => self.power_save = false,
            0xe2 => {
                self.page = 0;
                self.column_byte = 0;
                self.start_line = 0;
                self.saved_column = None;
                self.regulator = 0;
                self.contrast = 32;
                self.gray_mode = 0;
                self.palette = [0, 0, 0, 0, 255, 255, 255, 255];
                self.pending = None;
            }
            0xee => {
                if let Some(column) = self.saved_column.take() {
                    self.column_byte = column;
                }
            }
            0xe3 => {}
            0xe4 => self.inversion_lines = 0,
            0x20..=0x27 => self.regulator = byte & 7,
            0x28..=0x2f => self.power_control = byte & 7,
            0x64..=0x67 | 0x6c..=0x6f => self.booster = byte & 11,
            0x50..=0x57 => self.bias = byte & 7,
            0x90..=0x97 => self.gray_mode = byte & 7,
            // E8 belongs to the unbonded three-wire interface. Unassigned and
            // factory-test bytes have no modeled effect on this board.
            _ => {}
        }
        let _ = output.event(Event::LcdControl {
            at: now,
            command: byte,
            parameter: None,
        });
        Ok(())
    }
    fn shade(&self, row: usize, segment: usize) -> u8 {
        if self.entire_on {
            return 3;
        }
        let column = if self.segment_reverse {
            127 - segment
        } else {
            segment
        };
        let (high, low) = if row == 128 {
            (self.icons[2 * column], self.icons[2 * column + 1])
        } else {
            let index = (row / 8) * 256 + column * 2;
            (
                self.ram[index] >> (row & 7),
                self.ram[index + 1] >> (row & 7),
            )
        };
        let shade = ((high & 1) << 1) | (low & 1);
        if self.reverse {
            shade ^ 3
        } else {
            shade
        }
    }
}

impl Nt7508 {
    pub(crate) fn validate(&self, now: Time) -> Result<(), Error> {
        crate::state::require(
            self.bits < 8
                && self.page <= 16
                && self.column_byte < 256
                && self.saved_column.is_none_or(|v| v < 256)
                && self.start_line < 128
                && self.initial_com < 128
                && (16..=128).contains(&self.duty)
                && (self.inversion_lines == 0 || (3..=33).contains(&self.inversion_lines))
                && self.oscillator_frequency <= 31
                && self.gray_mode <= 7
                && self.contrast <= 63
                && self.oscillator_control <= 3,
            "invalid LCD controller",
        )?;
        self.scan.validate(now)?;
        crate::state::future(self.ready, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supply_domains_retain_ram_and_separate_scan_from_analog_drive() {
        let mut lcd = Nt7508::new();
        for command in [0xa5, 0xaf, 0xab] {
            lcd.write_counted_fixture(false, command, Time::ZERO, &mut ())
                .unwrap();
        }
        lcd.write_counted_fixture(true, 0xa5, Time::ZERO, &mut ())
            .unwrap();
        let mut reference = lcd.clone();
        lcd.set_supply(2000, Time::from_micros(100)).unwrap();
        assert_eq!(lcd.drive(Time::from_micros(200)).unwrap(), LcdDrive::OFF);
        lcd.set_supply(3000, Time::from_micros(300)).unwrap();
        assert_eq!(
            lcd.drive(Time::from_micros(400)).unwrap(),
            reference.drive(Time::from_micros(400)).unwrap()
        );
        assert_eq!(lcd.ram()[0], 0xa5);
        lcd.set_supply(1600, Time::from_micros(400)).unwrap();
        lcd.set_supply(3000, Time::from_micros(1400)).unwrap();
        assert_eq!(lcd.ram()[0], 0xa5);
        assert!(lcd.enabled()); // Nonzero dip does not invent RESETB.
        lcd.set_supply(0, Time::from_micros(1500)).unwrap();
        lcd.set_supply(3000, Time::from_micros(2500)).unwrap();
        assert_eq!(lcd.ram()[0], 0xa5);
        assert!(!lcd.enabled()); // Real rail removal invokes board RESETB.
        lcd.write_counted_fixture(false, 0xaf, Time::from_micros(2510), &mut ())
            .unwrap();
        lcd.set_supply(1600, Time::from_micros(2510)).unwrap();
        lcd.set_supply(3000, Time::from_micros(2511)).unwrap();
        lcd.write_counted_fixture(false, 0xaf, Time::from_micros(2525), &mut ())
            .unwrap();
        assert_eq!(
            lcd.deadline(),
            Time::from_micros(2511).checked_add(Duration::from_micros(21))
        );
        lcd.at_deadline(lcd.deadline().unwrap());
        assert!(!lcd.enabled(), "command inside the RESETB hold was ignored");
        reference.lose_volatile();
        assert_eq!(reference.ram()[0], 0);
    }
    fn commands(lcd: &mut Nt7508, bytes: &[u8]) {
        for &byte in bytes {
            lcd.write_counted_fixture(false, byte, Time::ZERO, &mut ())
                .unwrap();
        }
    }
    fn data(lcd: &mut Nt7508, bytes: &[u8]) {
        for &byte in bytes {
            lcd.write_counted_fixture(true, byte, Time::ZERO, &mut ())
                .unwrap();
        }
    }
    #[test]
    fn parameter_survives_chip_select() {
        let mut l = Nt7508::new();
        l.write_counted_fixture(false, 0x81, Time::ZERO, &mut ())
            .unwrap();
        l.write_counted_fixture(false, 0x35, Time::ZERO, &mut ())
            .unwrap();
        assert_eq!(l.contrast(), 0x35);
    }
    #[test]
    fn complete_bitplanes_and_nonvisible_ram_survive_reset() {
        let mut l = Nt7508::new();
        for c in [0xb8, 0x10, 0] {
            l.write_counted_fixture(false, c, Time::ZERO, &mut ())
                .unwrap();
        }
        l.write_counted_fixture(true, 1, Time::ZERO, &mut ())
            .unwrap();
        l.write_counted_fixture(true, 1, Time::ZERO, &mut ())
            .unwrap();
        for c in [0x44, 32, 0x40, 64, 0x8e, 0x99, 0x8f, 0x99, 0xab, 0xaf] {
            l.write_counted_fixture(false, c, Time::ZERO, &mut ())
                .unwrap();
        }
        let mut pixels = [0; LCD_WIDTH * LCD_HEIGHT];
        l.render(&mut pixels);
        assert_eq!(pixels[0], 255);
        l.write_counted_fixture(false, 0xe2, Time::ZERO, &mut ())
            .unwrap();
        assert_eq!(l.ram()[2048], 1);
    }
    #[test]
    fn icon_enable_is_independent_of_the_ram_page_and_only_db0_is_stored() {
        let mut lcd = Nt7508::new();
        commands(&mut lcd, &[0xa3]);
        data(&mut lcd, &[0xff, 0xfe]);
        commands(&mut lcd, &[0xb0]);
        data(&mut lcd, &[0x55]);
        assert!(lcd.icon_enabled);
        assert_eq!(lcd.icons()[..3], [1, 0, 0]);
        assert_eq!(lcd.ram()[2], 0x55);
        commands(&mut lcd, &[0xa3, 0xa2]);
        data(&mut lcd, &[0xff]);
        assert!(!lcd.icon_enabled);
        assert_eq!(lcd.icons()[3], 1);
    }
    #[test]
    fn controller_mapping_precedes_viewport_and_duty_gates_entire_on() {
        let mut lcd = Nt7508::new();
        commands(
            &mut lcd,
            &[
                0x44, 32, 0x48, 16, 0xaf, 0xab, 0x8a, 0x33, 0x8b, 0x33, 0x8c, 0x66, 0x8d, 0x66,
                0x8e, 0x99, 0x8f, 0x99,
            ],
        );
        data(&mut lcd, &[1, 0, 0, 1, 1, 1]);
        let mut pixels = [0; LCD_WIDTH * LCD_HEIGHT];
        lcd.render(&mut pixels);
        assert_eq!(&pixels[..4], &[170, 85, 255, 0]);
        commands(&mut lcd, &[0x17, 0x0f]);
        data(&mut lcd, &[1, 0]);
        commands(&mut lcd, &[0xa1]);
        lcd.render(&mut pixels);
        assert_eq!(&pixels[..3], &[170, 0, 0]);
        commands(&mut lcd, &[0xa7, 0xa5, 0x48, 0xff]);
        lcd.render(&mut pixels);
        assert!(pixels[..16 * LCD_WIDTH].iter().all(|&shade| shade == 255));
        assert!(pixels[16 * LCD_WIDTH..].iter().all(|&shade| shade == 0));
        commands(&mut lcd, &[0xae]);
        lcd.render(&mut pixels);
        assert!(pixels.iter().all(|&shade| shade == 0));
    }
    #[test]
    fn rendering_uses_palette_widths_and_only_the_selected_frc_frames() {
        let mut lcd = Nt7508::new();
        commands(
            &mut lcd,
            &[
                0x44, 32, 0x48, 64, 0xaf, 0xab, 0x88, 0x00, 0x89, 0x0f, 0x8a, 0x55, 0x8b, 0xf5,
                0x8c, 0xaa, 0x8d, 0xfa, 0x8e, 0xff, 0x8f, 0xff,
            ],
        );
        data(&mut lcd, &[0, 0, 0, 1, 1, 0, 1, 1]);
        let mut pixels = [0; LCD_WIDTH * LCD_HEIGHT];
        // These include invalid pulse widths, which turn off rather than clamp,
        // and a fourth frame whose widths differ from the preceding three.
        for (mode, expected) in [
            (0x95, [0, 142, 0, 0]),
            (0x96, [0, 106, 213, 0]),
            (0x97, [85, 85, 170, 255]),
            (0x91, [0, 106, 0, 0]),
            (0x92, [0, 80, 159, 0]),
            (0x93, [64, 128, 191, 255]),
        ] {
            commands(&mut lcd, &[mode]);
            lcd.render(&mut pixels);
            assert_eq!(&pixels[..4], &expected, "mode {mode:02x}");
        }
        commands(&mut lcd, &[0xa7]);
        lcd.render(&mut pixels);
        assert_eq!(&pixels[..4], &[255, 191, 128, 64]);
        commands(&mut lcd, &[0xa5]);
        lcd.render(&mut pixels);
        assert!(pixels.iter().all(|&pixel| pixel == 255));
        commands(&mut lcd, &[0xf7, 1]); // OSC1 is undriven on this board.
        lcd.render(&mut pixels);
        assert!(pixels.iter().all(|&pixel| pixel == 0));
    }
    #[test]
    fn software_reset_preserves_drive_configuration_and_unassigned_bytes_do_not_eat_commands() {
        let mut lcd = Nt7508::new();
        commands(
            &mut lcd,
            &[
                0x44, 32, 0x48, 64, 0xa1, 0xcf, 0xa3, 0xaf, 0xab, 0x2f, 0x95, 0xf6, 0x0a, 0x81, 8,
                0xe2,
            ],
        );
        assert!(lcd.icon_enabled && lcd.oscillator_enabled && lcd.enabled());
        assert!(lcd.segment_reverse && lcd.common_reverse);
        assert_eq!((lcd.initial_com, lcd.duty, lcd.power_control), (32, 64, 7));
        assert_eq!(
            (
                lcd.page,
                lcd.contrast,
                lcd.gray_mode,
                lcd.oscillator_frequency
            ),
            (0, 32, 0, 10)
        );
        commands(&mut lcd, &[0xe8, 0xae, 0xf0, 0xaf]);
        assert!(lcd.enabled());
        assert!(lcd.pending.is_none());
    }
}
