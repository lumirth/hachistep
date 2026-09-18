//! Package latches and the fixed board's digital connections. Pin-function
//! selection is kept separate from the output latch and resolved input level.
use super::ssu::Pins;
use crate::{
    error::Error,
    signals::{Buttons, DigitalPin, Drive},
};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Gpio {
    pub pfcr: u8,
    pmr: [u8; 3],
    latch: [u8; 5],
    direction: [u8; 4],
    pull: [u8; 4],
    open_drain9: u8,
    buttons: Buttons,
    analog_levels: [Option<bool>; 7],
    digital_levels: [Option<bool>; 11],
    aec_pwm: Option<bool>,
    aec_pwm_enabled: bool,
    clock_output: bool,
    clock_output_floating: bool,
    sci: super::sci::Pins,
    iic: Option<[bool; 2]>,
    incident_light: bool,
    pub levels: [u8; 5],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialLevels {
    pub lcd_selected: bool,
    pub data: bool,
    pub eeprom_selected: bool,
    pub sensor_selected: bool,
    pub clock: bool,
    pub mosi: bool,
}
impl Default for SerialLevels {
    fn default() -> Self {
        Self {
            lcd_selected: false,
            data: false,
            eeprom_selected: false,
            sensor_selected: false,
            clock: true,
            mosi: false,
        }
    }
}
impl Gpio {
    /// Resolve the board's pull resistors and fixture drivers with all MCU
    /// output stages unavailable, without discarding their retained latches.
    pub(crate) fn resolve_unpowered(&mut self, external_data: [Option<bool>; 2]) -> SerialLevels {
        let mut pins = Self {
            buttons: self.buttons,
            analog_levels: self.analog_levels,
            digital_levels: self.digital_levels,
            incident_light: self.incident_light,
            ..Self::default()
        };
        let levels = pins.resolve(Pins::default(), 0, 0, external_data);
        self.levels = pins.levels;
        levels
    }
    pub fn set_iic_pins(&mut self, pins: Option<[bool; 2]>) {
        self.iic = pins;
    }
    pub fn iic_inputs(&self) -> [bool; 2] {
        [self.levels[3] & 1 != 0, self.levels[3] & 2 != 0]
    }
    pub fn clock_selection(&self) -> u8 {
        self.pmr[0] & 7
    }
    pub fn set_clock_output(&mut self, level: Option<bool>, floating: bool) {
        if let Some(level) = level {
            self.clock_output = level;
        }
        self.clock_output_floating = floating;
    }
    pub fn set_buttons(&mut self, buttons: Buttons) {
        self.buttons = buttons;
    }
    pub fn adc_trigger(&self) -> (bool, bool) {
        (
            self.pmr[2] & 8 != 0,
            self.digital_levels[DigitalPin::Adtrg.index()].unwrap_or(false),
        )
    }
    pub fn set_digital_level(&mut self, pin: DigitalPin, level: Option<bool>) {
        self.digital_levels[pin.index()] = level;
    }
    pub fn set_aec_output(&mut self, enabled: bool, level: Option<bool>) {
        self.aec_pwm_enabled = enabled;
        self.aec_pwm = level;
    }
    pub fn set_sci_pins(&mut self, pins: super::sci::Pins, incident_light: bool) {
        self.sci = pins;
        self.incident_light = incident_light;
    }
    pub fn sci_inputs(&self) -> (Option<bool>, bool) {
        let clock = (self.pfcr & 3 != 2 && self.pmr[1] & 1 == 0).then_some(self.levels[1] & 1 != 0);
        (clock, self.levels[1] & 2 != 0)
    }
    pub fn emitting(&self) -> bool {
        self.levels[1] & 5 == 4
    }
    pub fn aec_inputs(&self) -> [Option<bool>; 3] {
        [
            (self.pmr[0] & 7 == 1).then_some(self.levels[0] & 1 != 0),
            (self.pmr[0] & 0x18 == 8 && self.pfcr & 0x0c != 8).then_some(self.levels[0] & 2 != 0),
            (self.pmr[0] & 0x20 != 0 && !self.aec_pwm_enabled).then_some(self.levels[0] & 4 != 0),
        ]
    }
    pub fn set_analog_levels(&mut self, levels: [Option<bool>; 7]) {
        self.analog_levels = levels;
    }
    pub fn raw_button_levels(&self) -> u8 {
        u8::from(self.buttons.center)
            | (u8::from(self.buttons.left) << 2)
            | (u8::from(self.buttons.right) << 4)
    }
    pub fn external_reference_selected(&self) -> bool {
        self.pmr[1] & 1 != 0
    }
    pub fn reset(&mut self) {
        let buttons = self.buttons;
        let analog_levels = self.analog_levels;
        let digital_levels = self.digital_levels;
        *self = Self::default();
        self.buttons = buttons;
        self.analog_levels = analog_levels;
        self.digital_levels = digital_levels;
    }
    pub fn handles(a: u16) -> bool {
        matches!(
            a,
            0xf085
                ..=0xf087
                    | 0xf08c
                    | 0xffc0
                    | 0xffc2
                    | 0xffca
                    | 0xffd4
                    | 0xffd6
                    | 0xffdb
                    | 0xffdc
                    | 0xffde
                    | 0xffe0
                    | 0xffe1
                    | 0xffe4
                    | 0xffe6
                    | 0xffeb
                    | 0xffec
        )
    }
    fn port_read(&self, index: usize, mask: u8) -> u8 {
        // This device family reads an output latch for output-configured pins.
        (self.latch[index] & self.direction[index] | self.levels[index] & !self.direction[index])
            & mask
    }
    pub fn read(&self, a: u16) -> u8 {
        match a {
            0xf085 => self.pfcr,
            0xf086 => self.pull[2],
            0xf087 => self.pull[3],
            0xf08c => self.open_drain9,
            0xffc0 => self.pmr[0],
            0xffc2 => self.pmr[1],
            0xffca => self.pmr[2],
            0xffd4 => self.port_read(0, 7),
            0xffd6 => self.port_read(1, 7),
            0xffdb => self.port_read(2, 0x1c),
            0xffdc => self.port_read(3, 15),
            0xffde => self.levels[4],
            0xffe0 => self.pull[0],
            0xffe1 => self.pull[1],
            // PCR readback is the selected deterministic witness for firmware's
            // read-modify-write instructions; unused package bits read zero.
            0xffe4 => self.direction[0],
            0xffe6 => self.direction[1],
            0xffeb => self.direction[2],
            0xffec => self.direction[3],
            _ => 0,
        }
    }
    pub fn write(&mut self, a: u16, v: u8) -> Result<(), Error> {
        match a {
            0xf085 => self.pfcr = v & 31,
            0xf086 => self.pull[2] = v & 0x1c,
            0xf087 => self.pull[3] = v & 15,
            0xf08c => self.open_drain9 = v & 15,
            0xffc0 => self.pmr[0] = v & 0x3f,
            0xffc2 => self.pmr[1] = v & 1,
            0xffca => self.pmr[2] = v & 0x0b,
            0xffd4 => self.latch[0] = v & 7,
            0xffd6 => self.latch[1] = v & 7,
            0xffdb => self.latch[2] = v & 0x1c,
            0xffdc => self.latch[3] = v & 15,
            0xffde => {} // Input-only port; writes have no electrical effect.
            0xffe0 => self.pull[0] = v & 7,
            0xffe1 => self.pull[1] = v & 7,
            0xffe4 => self.direction[0] = v & 7,
            0xffe6 => self.direction[1] = v & 7,
            0xffeb => self.direction[2] = v & 0x1c,
            0xffec => self.direction[3] = v & 15,
            _ => {
                return Err(Error::Unmapped {
                    address: a,
                    write: true,
                    width: 1,
                })
            }
        }
        Ok(())
    }
    /// Resolve the pins actually bonded to the shared serial devices. The
    /// default pull policy for unconnected/undriven inputs is documented in
    /// STATUS.md; no undocumented contention model is synthesized here.
    pub fn resolve(
        &mut self,
        serial: Pins,
        timer_levels: u8,
        timer_mask: u8,
        external_data: [Option<bool>; 2],
    ) -> SerialLevels {
        for i in 0..4 {
            self.levels[i] = self.latch[i] & self.direction[i] | self.pull[i] & !self.direction[i];
        }
        // Board chip-select lines idle high when not actively driven.
        self.levels[0] |= (!self.direction[0]) & 5;
        self.levels[3] |= (!self.direction[3]) & 1;
        self.levels[4] = self.raw_button_levels();
        for i in 0..6 {
            if let Some(high) = self.analog_levels[i] {
                self.levels[4] = (self.levels[4] & !(1 << i)) | (u8::from(high) << i);
            }
        }
        if let Some(high) = self.analog_levels[6] {
            self.levels[1] = (self.levels[1] & !1) | u8::from(high);
        }
        self.levels[2] =
            (self.levels[2] & !(timer_mask & 0x1c)) | (timer_levels & timer_mask & 0x1c);
        if self.pmr[0] & 7 == 0 && timer_mask & 2 != 0 {
            self.levels[0] = (self.levels[0] & !1) | ((timer_levels >> 1) & 1);
        }
        // P1 alternate input functions override PCR direction. GPIO input
        // fixtures also use this path; they cannot overwrite an active output.
        for i in 0..3 {
            let alternate = match i {
                0 => self.pmr[0] & 7 == 1,
                1 => self.pmr[0] & 0x18 != 0 || self.pfcr & 0x0c == 8,
                _ => self.pmr[0] & 0x20 != 0,
            };
            let timer_output = i == 0 && self.pmr[0] & 7 == 0 && timer_mask & 2 != 0;
            if alternate || (!timer_output && self.direction[0] & (1 << i) == 0) {
                let pull = if alternate {
                    self.pull[0] & !self.direction[0] & (1 << i) != 0 || (1 << i) & 5 != 0
                } else {
                    self.levels[0] & (1 << i) != 0
                };
                let high = if i == 2 && alternate && self.aec_pwm_enabled {
                    self.aec_pwm.unwrap_or(pull)
                } else {
                    self.digital_levels[i].unwrap_or(pull)
                };
                self.levels[0] = (self.levels[0] & !(1 << i)) | (u8::from(high) << i);
            }
        }
        if self.clock_selection() >= 2 {
            let high = if self.clock_output_floating {
                self.digital_levels[0].unwrap_or(true)
            } else {
                self.clock_output
            };
            self.levels[0] = (self.levels[0] & !1) | u8::from(high);
        }
        // P30: IRQ0, then VCref, then SCI clock, then GPIO. P31's SCI
        // input selection overrides PCR31. P32's SPC3 selects the peripheral
        // output independently of TE/PCR32. PCR output readback still uses
        // the original port latch (port_read), not these resolved voltages.
        let irq0 = self.pfcr & 3 == 2;
        let reference = !irq0 && self.pmr[1] & 1 != 0;
        let clock = if irq0 || reference {
            Some(Drive::Floating)
        } else {
            self.sci.clock
        };
        let clock = clock.unwrap_or(if self.direction[1] & 1 == 0 {
            Drive::Floating
        } else if self.latch[1] & 1 != 0 {
            Drive::High
        } else {
            Drive::Low
        });
        let high = match clock {
            Drive::Low => false,
            Drive::High => true,
            Drive::Floating => self.digital_levels[3]
                .or(if reference {
                    self.analog_levels[6]
                } else {
                    None
                })
                .unwrap_or(self.pull[1] & !self.direction[1] & 1 != 0),
        };
        self.levels[1] = (self.levels[1] & !1) | u8::from(high);
        if self.sci.receive || self.direction[1] & 2 == 0 {
            let receive = self.digital_levels[4].unwrap_or(high || !self.incident_light);
            self.levels[1] = (self.levels[1] & !2) | (u8::from(receive) << 1);
        }
        if let Some(tx) = self.sci.transmit {
            self.levels[1] = (self.levels[1] & !4) | (u8::from(tx) << 2);
        } else if self.direction[1] & 4 == 0 {
            if let Some(tx) = self.digital_levels[5] {
                self.levels[1] = (self.levels[1] & !4) | (u8::from(tx) << 2);
            }
        }
        for bit in 0..4 {
            let mask = 1 << bit;
            let function = if self.pfcr & 0x10 == 0 { bit } else { 3 - bit };
            let irq = (bit == 2 && self.pfcr & 3 == 1) || (bit == 3 && self.pfcr & 0x0c == 4);
            let mut drive = if irq {
                Drive::Floating
            } else {
                serial.drives[function].unwrap_or_else(|| {
                    if let Some(pins) = self.iic.filter(|_| bit < 2) {
                        if pins[bit] {
                            Drive::Floating
                        } else {
                            Drive::Low
                        }
                    } else if self.direction[3] & mask == 0 {
                        Drive::Floating
                    } else if self.latch[3] & mask == 0 {
                        Drive::Low
                    } else if self.open_drain9 & mask != 0 {
                        Drive::Floating
                    } else {
                        Drive::High
                    }
                })
            };
            if serial.data_open_drain && function >= 2 && drive == Drive::High {
                drive = Drive::Floating;
            }
            let high = match drive {
                Drive::Low => false,
                Drive::High => true,
                Drive::Floating => self.digital_levels[6 + bit]
                    .or(if bit >= 2 {
                        external_data[bit - 2]
                    } else {
                        None
                    })
                    .unwrap_or(
                        bit == 0 || (self.pull[3] & mask != 0 && self.direction[3] & mask == 0),
                    ),
            };
            self.levels[3] = (self.levels[3] & !mask) | u8::from(high) << bit;
        }
        SerialLevels {
            lcd_selected: self.levels[0] & 1 == 0,
            data: self.levels[0] & 2 != 0,
            eeprom_selected: self.levels[0] & 4 == 0,
            sensor_selected: self.levels[3] & 1 == 0,
            clock: self.levels[3] & 2 != 0,
            mosi: self.levels[3] & 4 != 0,
        }
    }
    /// Package inputs for Timer W. FTIOA is P10; B/C/D are P82/83/84.
    /// Capture can observe a GPIO output on a dual-purpose pin. FTCI is P11
    /// only when selected and not overridden by the IRQ1 route.
    pub fn timer_inputs(&self) -> [Option<bool>; 5] {
        [
            (self.pmr[0] & 7 == 0).then_some(self.levels[0] & 1 != 0),
            Some(self.levels[2] & 4 != 0),
            Some(self.levels[2] & 8 != 0),
            Some(self.levels[2] & 16 != 0),
            (self.pmr[0] & 0x10 != 0 && self.pfcr & 0x0c != 8).then_some(self.levels[0] & 2 != 0),
        ]
    }
    pub fn serial_inputs(&self) -> [bool; 4] {
        std::array::from_fn(|function| {
            let bit = if self.pfcr & 0x10 == 0 {
                function
            } else {
                3 - function
            };
            self.levels[3] & (1 << bit) != 0
        })
    }
    pub fn irq_levels(&self) -> [Option<bool>; 2] {
        [
            match self.pfcr & 3 {
                0 if self.pmr[2] & 1 != 0 => Some(self.levels[4] & 1 != 0),
                1 => Some(self.levels[3] & 4 != 0),
                2 => Some(self.levels[1] & 1 != 0),
                _ => None,
            },
            match (self.pfcr >> 2) & 3 {
                0 if self.pmr[2] & 2 != 0 => Some(self.levels[4] & 2 != 0),
                1 => Some(self.levels[3] & 8 != 0),
                2 => Some(self.levels[0] & 2 != 0),
                _ => None,
            },
        ]
    }
    pub(crate) fn irq_routes(&self) -> [u8; 2] {
        [
            match self.pfcr & 3 {
                0 if self.pmr[2] & 1 != 0 => 1, // PB0
                1 => 2,                         // P92
                2 => 3,                         // P30
                _ => 0,
            },
            match (self.pfcr >> 2) & 3 {
                0 if self.pmr[2] & 2 != 0 => 1, // PB1
                1 => 2,                         // P93
                2 => 3,                         // P11
                _ => 0,
            },
        ]
    }
    pub fn piezo_levels(&self) -> (bool, bool) {
        (self.levels[2] & 4 != 0, self.levels[2] & 8 != 0)
    }
    pub fn battery_switch(&self, timer_drive: bool) -> bool {
        self.levels[2] & 16 != 0 && (timer_drive || self.direction[2] & 16 != 0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssu_selection_overrides_iic_even_when_its_driver_is_released() {
        let mut g = Gpio::default();
        g.write(0xf087, 3).unwrap();
        g.set_iic_pins(Some([false, false]));
        g.resolve(Pins::default(), 0, 0, [None; 2]);
        assert_eq!(g.iic_inputs(), [false; 2]);
        let serial = Pins {
            drives: [Some(Drive::Floating), Some(Drive::Floating), None, None],
            data_open_drain: false,
        };
        g.resolve(serial, 0, 0, [None; 2]);
        assert_eq!(g.iic_inputs(), [true; 2]);
        // SSUS moves SSU's SCS/SSCK; unselected P90/P91 return to IIC.
        g.write(0xf085, 0x10).unwrap();
        g.resolve(serial, 0, 0, [None; 2]);
        assert_eq!(g.iic_inputs(), [false; 2]);
    }

    #[test]
    fn alternate_input_does_not_override_the_pullup_enable_condition() {
        let mut g = Gpio::default();
        for (a, v) in [(0xffd6, 1), (0xffe6, 1), (0xffe1, 1), (0xf085, 2)] {
            g.write(a, v).unwrap();
        }
        g.resolve(Pins::default(), 0, 0, [None; 2]);
        assert_eq!(g.irq_levels()[0], Some(false));
        assert_eq!(
            g.read(0xffd6) & 1,
            1,
            "PCR output readback still uses the latch"
        );
        g.write(0xffe6, 0).unwrap();
        g.resolve(Pins::default(), 0, 0, [None; 2]);
        assert_eq!(g.irq_levels()[0], Some(true));
    }

    #[test]
    fn button_pins_and_mux_are_not_firmware_flags() {
        let mut p = Gpio::default();
        p.set_buttons(Buttons {
            left: true,
            center: false,
            right: true,
        });
        p.write(0xffca, 1).unwrap();
        p.resolve(Default::default(), 0, 0, [None; 2]);
        assert_eq!(p.read(0xffde) & 0x15, 0x14);
        assert_eq!(p.irq_levels()[0], Some(false));
    }
    #[test]
    fn chip_selects_have_independent_lifetimes() {
        let mut p = Gpio::default();
        p.write(0xffe4, 7).unwrap();
        p.write(0xffd4, 4).unwrap();
        let s = p.resolve(
            Pins {
                drives: [
                    None,
                    Some(Drive::High),
                    Some(Drive::High),
                    Some(Drive::Floating),
                ],
                data_open_drain: false,
            },
            0,
            0,
            [None, Some(false)],
        );
        assert!(s.lcd_selected);
        assert!(!s.eeprom_selected);
        assert!(!s.data);
        assert!(!p.serial_inputs()[3]);
    }
}
