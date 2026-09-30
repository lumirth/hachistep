//! Package latches and the fixed board's digital connections. Pin-function
//! selection is kept separate from the output latch and resolved input level.
use super::ssu::Pins;
use crate::serial::{mask, Drives};
use crate::{
    error::Error,
    signals::{Buttons, DigitalPin, Drive},
};

pub(crate) const P1: u8 = 1 << 0;
pub(crate) const P3: u8 = 1 << 1;
pub(crate) const P8: u8 = 1 << 2;
pub(crate) const P9: u8 = 1 << 3;
pub(crate) const PB: u8 = 1 << 4;
pub(crate) const ALL_PORTS: u8 = P1 | P3 | P8 | P9 | PB;

/// Disposable electrical routing for an interval without configuration or
/// fixture changes. Drives and settled pad levels remain hardware-owner state.
pub(crate) struct SerialRoute {
    sources: [u8; 4],
    fallback: [Drive; 4],
    fixtures: [Option<bool>; 4],
    pulls: u8,
    open_drain: u8,
    reversed: bool,
    pub irq_observer: bool,
}
impl SerialRoute {
    fn resolve(&self, serial: Pins, external_data: [Option<bool>; 2]) -> u8 {
        let serial = serial
            .drives
            .map(|drive| drive.map(|drive| Drives::constant(drive, 1)));
        let external = external_data.map(|drive| {
            drive.map_or_else(Drives::default, |high| {
                Drives::constant(if high { Drive::High } else { Drive::Low }, 1)
            })
        });
        let planes = self.resolve_planes(serial, external, 1);
        (0..4).fold(0, |levels, bit| levels | ((planes[bit] & 1) as u8) << bit)
    }
    pub(crate) fn resolve_planes(
        &self,
        serial: [Option<Drives>; 4],
        external: [Drives; 2],
        count: u8,
    ) -> [u16; 4] {
        let lanes = mask(count);
        std::array::from_fn(|bit| self.resolve_plane(bit, serial, external, lanes))
    }
    pub(crate) fn resolve_plane(
        &self,
        bit: usize,
        serial: [Option<Drives>; 4],
        external: [Drives; 2],
        lanes: u16,
    ) -> u16 {
        let source = usize::from(self.sources[bit]);
        let fallback = Drives::constant(self.fallback[bit], lanes);
        let drive = if source < 4 {
            serial[source].unwrap_or(fallback)
        } else {
            fallback
        };
        let high = if self.open_drain & (1 << bit) == 0 {
            drive.high
        } else {
            0
        };
        let released = lanes & !(drive.low | high);
        let external = if bit >= 2 {
            external[bit - 2]
        } else {
            Drives::default()
        };
        let floating = !(external.low | external.high) & lanes;
        let passive = self.fixtures[bit].map_or_else(
            || {
                external.high
                    | if self.pulls & (1 << bit) != 0 {
                        floating
                    } else {
                        0
                    }
            },
            |high| if high { lanes } else { 0 },
        );
        high | released & passive
    }
    pub(crate) fn input_plane(&self, levels: [u16; 4], function: usize) -> u16 {
        levels[if self.reversed {
            3 - function
        } else {
            function
        }]
    }
    pub fn inputs(&self, levels: u8) -> [bool; 4] {
        std::array::from_fn(|function| {
            levels
                & (1 << if self.reversed {
                    3 - function
                } else {
                    function
                })
                != 0
        })
    }
}

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq, Default)]
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
    #[cfg(feature = "profile-work")]
    #[borsh(skip)]
    pub(crate) work: crate::profile_work::ResolutionWork,
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
    /// P9 input bits expose the live serial nets. Routing and direction reads
    /// expose retained configuration, independently of pending clock edges.
    pub(crate) fn reads_serial(address: u16) -> bool {
        address == 0xffdc
    }
    /// Resolve the board's pull resistors and fixture drivers with all MCU
    /// output stages unavailable, without discarding their retained latches.
    pub(crate) fn resolve_unpowered(&mut self, external_data: [Option<bool>; 2]) -> SerialLevels {
        #[cfg(feature = "profile-work")]
        self.work.board.add(1);
        let mut pins = Self {
            buttons: self.buttons,
            analog_levels: self.analog_levels,
            digital_levels: self.digital_levels,
            incident_light: self.incident_light,
            ..Self::default()
        };
        let levels = pins.resolve(Pins::default(), 0, 0, external_data);
        #[cfg(feature = "profile-work")]
        {
            self.work.serial.add(pins.work.serial.get());
            self.work.serial_lanes.add(pins.work.serial_lanes.get());
        }
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
        #[cfg(feature = "profile-work")]
        let work = self.work.clone();
        let buttons = self.buttons;
        let analog_levels = self.analog_levels;
        let digital_levels = self.digital_levels;
        *self = Self::default();
        #[cfg(feature = "profile-work")]
        {
            self.work = work;
        }
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
            // PCR reads return the direction latch, inferred from firmware's
            // read-modify-write instructions. Unused package bits read zero.
            0xffe4 => self.direction[0],
            0xffe6 => self.direction[1],
            0xffeb => self.direction[2],
            0xffec => self.direction[3],
            _ => 0,
        }
    }
    pub fn write(&mut self, a: u16, v: u8) -> Result<(), Error> {
        self.write_changed(a, v).map(|_| ())
    }
    /// Compare retained routing/latch state, not the pin's readback value.
    /// An input-configured pad can differ from its output latch.
    pub(crate) fn write_changed(&mut self, a: u16, v: u8) -> Result<bool, Error> {
        let (target, mask) = match a {
            0xf085 => (&mut self.pfcr, 31),
            0xf086 => (&mut self.pull[2], 0x1c),
            0xf087 => (&mut self.pull[3], 15),
            0xf08c => (&mut self.open_drain9, 15),
            0xffc0 => (&mut self.pmr[0], 0x3f),
            0xffc2 => (&mut self.pmr[1], 1),
            0xffca => (&mut self.pmr[2], 0x0b),
            0xffd4 => (&mut self.latch[0], 7),
            0xffd6 => (&mut self.latch[1], 7),
            0xffdb => (&mut self.latch[2], 0x1c),
            0xffdc => (&mut self.latch[3], 15),
            0xffde => return Ok(false), // Input-only port.
            0xffe0 => (&mut self.pull[0], 7),
            0xffe1 => (&mut self.pull[1], 7),
            0xffe4 => (&mut self.direction[0], 7),
            0xffe6 => (&mut self.direction[1], 7),
            0xffeb => (&mut self.direction[2], 0x1c),
            0xffec => (&mut self.direction[3], 15),
            _ => {
                return Err(Error::Unmapped {
                    address: a,
                    write: true,
                    width: 1,
                })
            }
        };
        let value = v & mask;
        let changed = *target != value;
        *target = value;
        Ok(changed)
    }
    /// Resolve the shared serial pins. Chip-select nets have a board pull-up;
    /// other released nets follow connected drivers and enabled MCU pull-ups,
    /// then default low. See docs/accuracy/bus-and-gpio.md.
    pub fn resolve(
        &mut self,
        serial: Pins,
        timer_levels: u8,
        timer_mask: u8,
        external_data: [Option<bool>; 2],
    ) -> SerialLevels {
        self.resolve_ports(serial, timer_levels, timer_mask, external_data, ALL_PORTS)
    }
    /// Resolve only ports whose drivers or routing changed. The other ports
    /// retain their settled physical levels; input consumers observe changes
    /// after all drivers of an affected net have been combined.
    pub(crate) fn resolve_ports(
        &mut self,
        serial: Pins,
        timer_levels: u8,
        timer_mask: u8,
        external_data: [Option<bool>; 2],
        ports: u8,
    ) -> SerialLevels {
        #[cfg(feature = "profile-work")]
        self.work.board.add(1);
        for i in 0..3 {
            if ports & (1 << i) != 0 {
                self.levels[i] =
                    self.latch[i] & self.direction[i] | self.pull[i] & !self.direction[i];
            }
        }
        if ports & PB != 0 {
            self.levels[4] = self.raw_button_levels();
            for i in 0..6 {
                if let Some(high) = self.analog_levels[i] {
                    self.levels[4] = (self.levels[4] & !(1 << i)) | (u8::from(high) << i);
                }
            }
        }
        if ports & P8 != 0 {
            self.levels[2] =
                (self.levels[2] & !(timer_mask & 0x1c)) | (timer_levels & timer_mask & 0x1c);
        }
        if ports & P1 != 0 {
            // Board chip-select lines idle high when not actively driven.
            self.levels[0] |= (!self.direction[0]) & 5;
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
        }
        if ports & P3 != 0 {
            if let Some(high) = self.analog_levels[6] {
                self.levels[1] = (self.levels[1] & !1) | u8::from(high);
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
        }
        if ports & P9 != 0 {
            self.resolve_serial(serial, external_data)
        } else {
            self.serial_levels()
        }
    }
    /// Resolve P9 after serial drivers change. Other ports retain their settled
    /// levels; `resolve_ports` updates other drivers independently.
    pub(crate) fn resolve_serial(
        &mut self,
        serial: Pins,
        external_data: [Option<bool>; 2],
    ) -> SerialLevels {
        let route = self.serial_route(serial);
        self.resolve_serial_route(&route, serial, external_data)
    }
    pub(crate) fn serial_route(&self, serial: Pins) -> SerialRoute {
        let mut route = SerialRoute {
            sources: [4; 4],
            fallback: [Drive::Floating; 4],
            fixtures: std::array::from_fn(|bit| self.digital_levels[6 + bit]),
            pulls: (self.pull[3] & !self.direction[3]) | 1,
            open_drain: 0,
            reversed: self.pfcr & 0x10 != 0,
            irq_observer: self.irq_routes().contains(&2),
        };
        for bit in 0..4 {
            let mask = 1 << bit;
            let function = if self.pfcr & 0x10 == 0 { bit } else { 3 - bit };
            let irq = (bit == 2 && self.pfcr & 3 == 1) || (bit == 3 && self.pfcr & 0x0c == 4);
            route.fallback[bit] = if irq {
                Drive::Floating
            } else {
                if serial.drives[function].is_some() {
                    route.sources[bit] = function as u8;
                }
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
            };
            if serial.data_open_drain && function >= 2 {
                route.open_drain |= mask;
            }
        }
        route
    }
    pub(crate) fn resolve_serial_route(
        &mut self,
        route: &SerialRoute,
        serial: Pins,
        external_data: [Option<bool>; 2],
    ) -> SerialLevels {
        #[cfg(feature = "profile-work")]
        {
            self.work.serial.add(1);
            self.work.serial_lanes.add(1);
        }
        self.levels[3] = route.resolve(serial, external_data);
        self.serial_levels()
    }
    pub(crate) fn serial_levels(&self) -> SerialLevels {
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

impl Gpio {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        crate::state::require(
            self.pfcr & !31 == 0
                && self.open_drain9 & !15 == 0
                && self
                    .pmr
                    .iter()
                    .zip([0x3f, 1, 0x0b])
                    .all(|(v, mask)| v & !mask == 0)
                && [self.direction, self.pull].iter().all(|a| {
                    a.iter()
                        .zip([7, 7, 0x1c, 15])
                        .all(|(v, mask)| v & !mask == 0)
                }),
            "invalid GPIO storage bits",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn released_data_planes_use_external_drivers_and_fixture_priority() {
        for fixture in [None, Some(false), Some(true)] {
            let mut gpio = Gpio::default();
            gpio.set_digital_level(DigitalPin::P92, fixture);
            let serial = Pins {
                drives: [None, None, Some(Drive::High), None],
                data_open_drain: true,
            };
            let route = gpio.serial_route(serial);
            let drives = [None, None, Some(Drives::constant(Drive::High, 15)), None];
            let external = [Drives { low: 3, high: 12 }, Drives::default()];
            let expected = match fixture {
                None => 12,
                Some(false) => 0,
                Some(true) => 15,
            };
            assert_eq!(route.resolve_planes(drives, external, 4)[2], expected);
            // A driven low is never overwritten by a passive/fixture level.
            let drives = [None, None, Some(Drives::constant(Drive::Low, 15)), None];
            assert_eq!(route.resolve_planes(drives, external, 4)[2], 0);
        }
    }

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
