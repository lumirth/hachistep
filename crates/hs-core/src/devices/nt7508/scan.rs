//! Analytic LCD scan. A stable interval has no machine-scheduler appointments;
//! only its last quantum's output latch needs to be materialized. Geometry is
//! latched per frame, PWM per row, and pixel/palette data per quantum.
use super::Nt7508;
use crate::{
    error::Error,
    time::{Clock, Time},
};

/// Digital controller drive before analog panel response. Bit n%64 of word n/64
/// describes SEGn's active level relative to the selected common. Common 128
/// is the icon output; it is outside the Pokéwalker's 64-row glass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LcdDrive {
    pub common: Option<u8>,
    pub segments: [u64; 2],
    pub inverted: bool,
}
impl LcdDrive {
    pub const OFF: Self = Self {
        common: None,
        segments: [0; 2],
        inverted: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scan {
    clock: Option<Clock>,
    rate: (u64, u64),
    duty: u8,
    icon: bool,
    initial_com: u8,
    common_reverse: bool,
    start_line: u8,
    pwm: u8,
    frc_count: u8,
    row: u8,
    step: u8,
    frc: u8,
    inversion_lines: u8,
    inversion_count: u8,
    inverted: bool,
    segments: [u64; 2],
}
impl Default for Scan {
    fn default() -> Self {
        Self {
            clock: None,
            rate: (129024, 1),
            duty: 128,
            icon: false,
            initial_com: 0,
            common_reverse: false,
            start_line: 0,
            pwm: 9,
            frc_count: 4,
            row: 0,
            step: 0,
            frc: 0,
            inversion_lines: 0,
            inversion_count: 0,
            inverted: false,
            segments: [0; 2],
        }
    }
}
impl Scan {
    fn rows(self) -> u64 {
        u64::from(self.duty) + u64::from(self.icon)
    }
    fn row_remaining(self) -> u64 {
        u64::from(self.pwm - self.step)
    }
    fn frame_remaining(self) -> u64 {
        (self.rows() - u64::from(self.row)) * u64::from(self.pwm) - u64::from(self.step)
    }
    fn frame_settings(&mut self, lcd: &Nt7508) {
        self.duty = lcd.duty;
        self.icon = lcd.icon_enabled;
        self.initial_com = lcd.initial_com;
        self.common_reverse = lcd.common_reverse;
        self.start_line = lcd.start_line;
        self.frc_count = lcd.frc_count();
    }
    fn frame_pending(self, lcd: &Nt7508) -> bool {
        self.duty != lcd.duty
            || self.icon != lcd.icon_enabled
            || self.initial_com != lcd.initial_com
            || self.common_reverse != lcd.common_reverse
            || self.start_line != lcd.start_line
            || self.frc_count != lcd.frc_count()
    }
    fn count(&mut self, quanta: u64, lcd: &Nt7508) {
        // u128 permits a large caller horizon without overflowing additions.
        let total = u128::from(self.step) + u128::from(quanta);
        let rows = total / u128::from(self.pwm);
        self.step = (total % u128::from(self.pwm)) as u8;
        let total_rows = u128::from(self.row) + rows;
        let frames = total_rows / u128::from(self.rows());
        self.row = (total_rows % u128::from(self.rows())) as u8;
        if self.inversion_lines == 0 {
            self.inverted ^= frames & 1 != 0;
        } else {
            let lines = u128::from(self.inversion_count) + rows;
            self.inverted ^= (lines / u128::from(self.inversion_lines)) & 1 != 0;
            self.inversion_count = (lines % u128::from(self.inversion_lines)) as u8;
        }
        if frames != 0 {
            self.frame_settings(lcd);
            self.frc = ((u128::from(self.frc) + frames) % u128::from(self.frc_count)) as u8;
        }
        if rows != 0 {
            self.pwm = lcd.pwm();
            if self.inversion_lines != lcd.inversion_lines {
                self.inversion_lines = lcd.inversion_lines;
                self.inversion_count = 0;
            }
        }
    }
}
impl Nt7508 {
    fn pwm(&self) -> u8 {
        [9, 9, 12, 15][usize::from(self.gray_mode & 3)]
    }
    fn frc_count(&self) -> u8 {
        if self.gray_mode & 4 == 0 {
            4
        } else {
            3
        }
    }
    fn scan_rate(&self, scan: Scan) -> (u64, u64) {
        if self.oscillator_control & 2 == 0 {
            (1008 * scan.rows(), 1)
        } else {
            (
                [92000, 122000, 147000, 184000][usize::from(self.oscillator_frequency >> 3)],
                u64::from(self.oscillator_frequency & 7) + 1,
            )
        }
    }
    fn latch(&self, scan: Scan) -> [u64; 2] {
        let widths: [u8; 4] = core::array::from_fn(|shade| {
            (self.palette[shade * 2 + usize::from(scan.frc / 2)] >> ((scan.frc & 1) * 4)) & 15
        });
        let active = widths.map(|width| width <= scan.pwm && scan.step < width);
        let row = if scan.row == scan.duty {
            128
        } else {
            usize::from(scan.start_line.wrapping_add(scan.row) & 127)
        };
        let mut segments = [0; 2];
        for segment in 0..128 {
            if active[usize::from(self.shade(row, segment))] {
                segments[segment / 64] |= 1 << (segment & 63);
            }
        }
        segments
    }
    pub(super) fn project_scan(&self, now: Time) -> Result<Scan, Error> {
        let mut scan = self.scan;
        let mut changed = false;
        while let Some(mut clock) = scan.clock {
            let mut edges = clock.edges_before(now);
            if edges != u64::MAX && clock.after(edges + 1).ok() == Some(now) {
                edges += 1;
            }
            if edges == 0 {
                break;
            }
            let boundary = if scan.rate != self.scan_rate(scan) {
                1 // Finish the already-running quantum at its old rate.
            } else if scan.pwm != self.pwm() || scan.inversion_lines != self.inversion_lines {
                scan.row_remaining()
            } else if scan.frame_pending(self) {
                scan.frame_remaining()
            } else {
                u64::MAX
            };
            let count = edges.min(boundary);
            let at = clock.advance(count)?;
            scan.count(count, self);
            let rate = self.scan_rate(scan);
            if rate != scan.rate {
                clock = Clock::new(at, rate.0, rate.1)?;
                scan.rate = rate;
            }
            scan.clock = Some(clock);
            changed = true;
        }
        if changed {
            scan.segments = self.latch(scan);
        }
        Ok(scan)
    }
    pub(super) fn update_clock(&mut self, now: Time, was_running: bool) -> Result<(), Error> {
        if self.inversion_lines == 0 {
            // Releasing n-line inversion retains M until the next frame; an
            // old line-divider expiration must not toggle it in between.
            self.scan.inversion_lines = 0;
            self.scan.inversion_count = 0;
        }
        let running = self.oscillator_enabled && !self.power_save;
        if running && !was_running {
            let mut scan = Scan::default();
            scan.frame_settings(self);
            scan.pwm = self.pwm();
            scan.inversion_lines = self.inversion_lines;
            scan.rate = self.scan_rate(scan);
            scan.segments = self.latch(scan);
            self.scan = scan;
        }
        if !running || self.oscillator_control & 1 != 0 {
            self.scan.clock = None;
        } else if self.scan.clock.is_none() {
            self.scan.rate = self.scan_rate(self.scan);
            self.scan.clock = Some(Clock::new(now, self.scan.rate.0, self.scan.rate.1)?);
        }
        Ok(())
    }
    /// Project through `now` without advancing guest execution or changing the
    /// retained state. Firmware can change RAM inside the current scan row.
    pub fn drive(&self, now: Time) -> Result<LcdDrive, Error> {
        if !self.enabled() || !self.oscillator_enabled {
            return Ok(LcdDrive::OFF);
        }
        let scan = self.project_scan(now)?;
        let common = if scan.row == scan.duty {
            128
        } else {
            let common = scan.initial_com.wrapping_add(scan.row) & 127;
            if scan.common_reverse {
                127 - common
            } else {
                common
            }
        };
        Ok(LcdDrive {
            common: Some(common),
            segments: scan.segments,
            inverted: scan.inverted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write(lcd: &mut Nt7508, at: Time, data: bool, values: &[u8]) {
        for &byte in values {
            lcd.write_counted_fixture(data, byte, at, &mut ()).unwrap();
        }
    }
    fn quantum(n: u64) -> Time {
        Time::from_raw((u128::from(n) << 64) / 92000)
    }
    fn configured(extra: &[u8]) -> Nt7508 {
        let mut lcd = Nt7508::new();
        write(
            &mut lcd,
            Time::ZERO,
            false,
            &[
                0x44, 32, 0x48, 16, 0xf7, 2, 0xf6, 0, 0x95, 0x8e, 0x21, 0x8f, 0x43, 0xa5, 0xaf,
            ],
        );
        write(&mut lcd, Time::ZERO, false, extra);
        write(&mut lcd, Time::ZERO, false, &[0xab]);
        lcd
    }
    #[test]
    fn palette_nibbles_and_frc_frames_produce_documented_pwm_widths() {
        let lcd = configured(&[]);
        for (edge, on) in [
            (0, true),
            (1, false),
            (144, true),
            (145, true),
            (146, false),
            (288, true),
            (290, true),
            (291, false),
            (432, true),
            (433, false),
        ] {
            assert_eq!(
                lcd.drive(quantum(edge)).unwrap().segments,
                [if on { u64::MAX } else { 0 }; 2]
            );
        }
        let mut lcd = configured(&[0x8e, 0xaa, 0x8f, 0xaa]);
        assert_eq!(lcd.drive(quantum(1)).unwrap().segments, [0; 2]);
        write(&mut lcd, quantum(1), false, &[0x96]); // 12 PWM begins with next row.
        assert_eq!(lcd.drive(quantum(9)).unwrap().segments, [u64::MAX; 2]);
        assert_eq!(lcd.drive(quantum(19)).unwrap().segments, [0; 2]);
        assert_eq!(lcd.drive(quantum(20)).unwrap().common, Some(33));
        assert_eq!(lcd.drive(quantum(21)).unwrap().common, Some(34));
    }
    #[test]
    fn start_line_latches_per_frame_but_ram_latches_per_quantum() {
        let mut lcd = configured(&[0xa4, 0x8c, 0x99, 0x8d, 0x99]);
        let halfway = Time::from_raw(quantum(1).raw() / 2);
        write(&mut lcd, halfway, true, &[0xff]);
        assert_eq!(lcd.drive(halfway).unwrap().segments[0], 0);
        assert_eq!(lcd.drive(quantum(1)).unwrap().segments[0], 1);
        write(&mut lcd, quantum(1), false, &[0x40, 64]);
        assert_eq!(lcd.drive(quantum(9)).unwrap().segments[0], 1);
        assert_eq!(lcd.drive(quantum(144)).unwrap().segments[0], 0);
        // A write tied with an edge follows the old output latch on that edge.
        write(&mut lcd, quantum(145), false, &[0xb8, 0x10, 0]);
        write(&mut lcd, quantum(145), true, &[1]);
        assert_eq!(lcd.drive(quantum(145)).unwrap().segments[0], 0);
        assert_eq!(lcd.drive(quantum(146)).unwrap().segments[0], 1);
    }
    #[test]
    fn n_line_inversion_carries_across_frame_wrap_and_chunking() {
        let lcd = configured(&[0x48, 128, 0x44, 0, 0x4c, 5]);
        for (line, common, inverted) in [
            (124, 124, false),
            (125, 125, true),
            (127, 127, true),
            (128, 0, true),
            (129, 1, true),
            (130, 2, false),
        ] {
            let drive = lcd.drive(quantum(line * 9)).unwrap();
            assert_eq!((drive.common, drive.inverted), (Some(common), inverted));
        }
        let mut released = lcd.clone();
        write(&mut released, quantum(36), false, &[0xe4]);
        assert!(!released.drive(quantum(45)).unwrap().inverted);
        assert!(released.drive(quantum(128 * 9)).unwrap().inverted);
        let mut split = lcd.clone();
        for edge in (1..10000).step_by(37) {
            split.scan = split.project_scan(quantum(edge)).unwrap();
        }
        assert_eq!(
            lcd.project_scan(quantum(10000)).unwrap(),
            split.project_scan(quantum(10000)).unwrap()
        );
        let saved = split.clone();
        write(&mut split, quantum(10000), false, &[0xa9]);
        assert_eq!(split.drive(quantum(20000)).unwrap(), LcdDrive::OFF);
        write(&mut split, quantum(20000), false, &[0xe1]);
        assert_eq!(split.drive(quantum(20000)).unwrap().common, Some(0));
        assert_eq!(
            saved.drive(quantum(20000)).unwrap(),
            lcd.drive(quantum(20000)).unwrap()
        );
    }
    #[test]
    fn fixed_rate_and_live_clock_changes_keep_the_remaining_quantum() {
        for (mode, common) in [(0x90, 32), (0x92, 32), (0x93, 35)] {
            let lcd = configured(&[mode, 0xf7, 0]);
            assert_eq!(
                lcd.drive(Time::from_micros(1_000_000)).unwrap().common,
                Some(common)
            );
        }
        let mut lcd = configured(&[]);
        let halfway = Time::from_raw(quantum(1).raw() / 2);
        write(&mut lcd, halfway, false, &[0xf6, 7]); // divide by eight after next edge.
        assert_eq!(lcd.drive(quantum(8)).unwrap().segments, [0; 2]);
        assert_eq!(lcd.drive(quantum(64)).unwrap().common, Some(32));
        assert_eq!(lcd.drive(quantum(65)).unwrap().common, Some(33));
        write(&mut lcd, quantum(65), false, &[0xf7, 3]); // OSC1 is undriven.
        assert_eq!(
            lcd.drive(quantum(66)).unwrap(),
            lcd.drive(quantum(100000)).unwrap()
        );
    }
}
