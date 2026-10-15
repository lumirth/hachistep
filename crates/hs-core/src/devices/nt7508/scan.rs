//! Analytic LCD scan. A stable interval has no machine-scheduler appointments;
//! only its last quantum's output latch needs to be materialized. Geometry is
//! latched per frame, PWM per row, and pixel/palette data per quantum. The
//! latch itself is materialized only before a change to what it latched.
use super::{Nt7508, LCD_HEIGHT, LCD_WIDTH};
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

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Whether `segments` holds the latch. Otherwise the latch is that of the
    /// controller's current RAM and settings, which have not changed since.
    latched: bool,
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
            latched: true,
        }
    }
}
impl Scan {
    pub(super) fn stop(&mut self) {
        self.clock = None;
    }
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
        // Most serial writes advance within the same scan row. No row or
        // frame latch changes there, so only the PWM quantum advances.
        if quanta < self.row_remaining() {
            self.step += quanta as u8;
            return;
        }
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
    fn pulse_width(&self, shade: usize, frame: u8, pwm: u8) -> u8 {
        let width = (self.palette[shade * 2 + usize::from(frame / 2)] >> ((frame & 1) * 4)) & 15;
        // NT7508 p.47: encodings beyond the selected PWM range produce no pulse.
        if width <= pwm {
            width
        } else {
            0
        }
    }
    fn latch(&self, scan: Scan) -> [u64; 2] {
        let active: [bool; 4] =
            core::array::from_fn(|shade| scan.step < self.pulse_width(shade, scan.frc, scan.pwm));
        let row = self.latched_row(scan);
        let mut segments = [0; 2];
        for segment in 0..128 {
            if active[usize::from(self.shade(row, segment))] {
                segments[segment / 64] |= 1 << (segment & 63);
            }
        }
        segments
    }
    /// Project the programmed image with each palette entry averaged over its
    /// FRC cycle. Values are normalized drive from 0 to 255. This view uses
    /// current settings; `drive` exposes the scan and latch at a specific time.
    pub fn render(&self, pixels: &mut [u8; LCD_WIDTH * LCD_HEIGHT]) {
        if !self.enabled() || !self.oscillator_enabled || self.oscillator_control & 1 != 0 {
            pixels.fill(0);
            return;
        }
        let pwm = self.pwm();
        let frames = self.frc_count();
        let period = u16::from(pwm) * u16::from(frames);
        let levels: [u8; 4] = core::array::from_fn(|shade| {
            let active: u16 = (0..frames)
                .map(|frame| u16::from(self.pulse_width(shade, frame, pwm)))
                .sum();
            ((active * 255 + period / 2) / period) as u8
        });
        for y in 0..LCD_HEIGHT {
            // The panel occupies SEG0..95 and COM32..95. Direction commands
            // address the complete 128-output controller before that crop.
            let common = if self.common_reverse { 95 - y } else { y + 32 };
            let line = (common + 128 - usize::from(self.initial_com)) & 127;
            for x in 0..LCD_WIDTH {
                pixels[y * LCD_WIDTH + x] = if line < usize::from(self.duty) {
                    levels[usize::from(self.shade((line + usize::from(self.start_line)) & 127, x))]
                } else {
                    0
                };
            }
        }
    }
    pub(super) fn project_scan(&self, now: Time) -> Result<Scan, Error> {
        let mut scan = self.scan;
        let mut changed = false;
        while let Some(mut clock) = scan.clock {
            let edges = match now.raw().checked_add(1) {
                Some(end) => clock.edges_before(Time::from_raw(end)),
                None => {
                    let edges = clock.edges_before(now);
                    edges + u64::from(edges != u64::MAX && clock.after(edges + 1).ok() == Some(now))
                }
            };
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
            let changed_rate = rate != scan.rate;
            if changed_rate {
                clock = Clock::new(at, rate.0, rate.1)?;
                scan.rate = rate;
            }
            scan.clock = Some(clock);
            changed = true;
            if count == edges && !changed_rate {
                break;
            }
        }
        if changed {
            scan.latched = false;
        }
        Ok(scan)
    }
    /// The output latch of the current quantum.
    fn segments(&self, scan: Scan) -> [u64; 2] {
        if scan.latched {
            scan.segments
        } else {
            self.latch(scan)
        }
    }
    /// Materialize the latch before a change to the RAM or settings it reads.
    /// A data byte only affects the latch when it lands in the latched row.
    pub(super) fn retain_latch(&mut self, data_page: Option<u8>) {
        if self.scan.latched {
            return;
        }
        if let Some(page) = data_page {
            let row = self.latched_row(self.scan);
            let affected = if page == 16 {
                row == 128
            } else {
                row != 128 && row / 8 == usize::from(page & 15)
            };
            if !affected {
                return;
            }
        }
        self.scan.segments = self.latch(self.scan);
        self.scan.latched = true;
    }
    fn latched_row(&self, scan: Scan) -> usize {
        if scan.row == scan.duty {
            128
        } else {
            usize::from(scan.start_line.wrapping_add(scan.row) & 127)
        }
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
            scan.latched = true;
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
            segments: self.segments(scan),
            inverted: scan.inverted,
        })
    }
}

impl Scan {
    pub(super) fn validate(&self, now: Time) -> Result<(), Error> {
        crate::state::require(
            self.rate.0 > 0
                && self.rate.1 > 0
                && (16..=128).contains(&self.duty)
                && self.initial_com < 128
                && self.start_line < 128
                && matches!(self.pwm, 9 | 12 | 15)
                && self.step < self.pwm
                && matches!(self.frc_count, 3 | 4)
                && self.frc < self.frc_count
                && u16::from(self.row) < u16::from(self.duty) + u16::from(self.icon)
                && (self.inversion_lines == 0 || (3..=33).contains(&self.inversion_lines))
                && (self.inversion_lines == 0 || self.inversion_count < self.inversion_lines),
            "invalid LCD scan",
        )?;
        if let Some(clock) = self.clock {
            clock.validate(now)?;
        }
        Ok(())
    }
}
