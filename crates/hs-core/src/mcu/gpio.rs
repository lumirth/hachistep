//! Package latches and the fixed board's digital connections. Pin-function
//! selection is kept separate from the output latch and resolved input level.
use crate::{error::Error, signals::Buttons};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gpio {
    target_f088: u8,
    pub pfcr: u8,
    pmr: [u8; 3],
    latch: [u8; 5],
    direction: [u8; 4],
    pull: [u8; 4],
    open_drain9: u8,
    buttons: Buttons,
    pub levels: [u8; 5],
}
impl Default for Gpio {
    fn default() -> Self {
        Self { target_f088: 0, pfcr: 0, pmr: [0; 3], latch: [0; 5], direction: [0; 4],
            pull: [0; 4], open_drain9: 0, buttons: Buttons::default(),
            levels: [0; 5] }
    }
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
        Self { lcd_selected: false, data: false, eeprom_selected: false,
            sensor_selected: false, clock: true, mosi: false }
    }
}
impl Gpio {
    pub fn set_buttons(&mut self, buttons: Buttons) { self.buttons = buttons; }
    pub fn reset(&mut self) {
        let buttons = self.buttons;
        *self = Self::default(); self.buttons = buttons;
    }
    pub fn handles(a: u16) -> bool {
        matches!(a, 0xf085..=0xf088 | 0xf08c | 0xffc0 | 0xffc2 | 0xffca |
            0xffd4 | 0xffd6 | 0xffdb | 0xffdc | 0xffde | 0xffe0 | 0xffe1 |
            0xffe4 | 0xffe6 | 0xffeb | 0xffec)
    }
    fn port_read(&self, index: usize, mask: u8) -> u8 {
        // This device family reads an output latch for output-configured pins.
        (self.latch[index] & self.direction[index] |
            self.levels[index] & !self.direction[index]) & mask
    }
    pub fn read(&self, a: u16) -> u8 {
        match a {
            0xf088 => self.target_f088,
            0xf085 => self.pfcr, 0xf086 => self.pull[2], 0xf087 => self.pull[3],
            0xf08c => self.open_drain9, 0xffc0 => self.pmr[0], 0xffc2 => self.pmr[1],
            0xffca => self.pmr[2], 0xffd4 => self.port_read(0,7),
            0xffd6 => self.port_read(1,7), 0xffdb => self.port_read(2,0x1c),
            0xffdc => self.port_read(3,15), 0xffde => self.levels[4],
            0xffe0 => self.pull[0], 0xffe1 => self.pull[1],
            // PCR readback is the selected deterministic witness for firmware's
            // read-modify-write instructions; unused package bits read zero.
            0xffe4 => self.direction[0], 0xffe6 => self.direction[1],
            0xffeb => self.direction[2], 0xffec => self.direction[3], _ => 0,
        }
    }
    pub fn write(&mut self, a: u16, v: u8) -> Result<(), Error> {
        match a {
            0xf088 => {
                if v & !3 != 0 {return Err(Error::Unsupported{component:"target F088",detail:"uncharacterized bits outside observed low two fields",address:a});}
                self.target_f088 = v;
            }
            0xf085 => self.pfcr = v & 31,
            0xf086 => self.pull[2] = v & 0x1c, 0xf087 => self.pull[3] = v & 15,
            0xf08c => self.open_drain9 = v & 15,
            0xffc0 => self.pmr[0] = v & 0x3f, 0xffc2 => self.pmr[1] = v & 1,
            0xffca => self.pmr[2] = v & 0x0b,
            0xffd4 => self.latch[0] = v & 7, 0xffd6 => self.latch[1] = v & 7,
            0xffdb => self.latch[2] = v & 0x1c, 0xffdc => self.latch[3] = v & 15,
            0xffde => {}, // Input-only port; writes have no electrical effect. 0xffe0 => self.pull[0] = v & 7, 0xffe1 => self.pull[1] = v & 7,
            0xffe4 => self.direction[0] = v & 7, 0xffe6 => self.direction[1] = v & 7,
            0xffeb => self.direction[2] = v & 0x1c, 0xffec => self.direction[3] = v & 15,
            _ => return Err(Error::Unmapped { address: a, write: true, width: 1 }),
        }
        Ok(())
    }
    /// Resolve the pins actually bonded to the shared serial devices. The
    /// default pull policy for unconnected/undriven inputs is documented in
    /// STATUS.md; no undocumented contention model is synthesized here.
    pub fn resolve(&mut self, serial: Option<(bool,bool)>, timer_levels: u8,
        timer_mask: u8, miso: Option<bool>) -> SerialLevels {
        for i in 0..4 {
            self.levels[i] = self.latch[i] & self.direction[i] | self.pull[i] & !self.direction[i];
        }
        // Board chip-select lines idle high when not actively driven.
        self.levels[0] |= (!self.direction[0]) & 5;
        self.levels[3] |= (!self.direction[3]) & 1;
        self.levels[4] = u8::from(self.buttons.center) |
            u8::from(self.buttons.left) << 2 | u8::from(self.buttons.right) << 4;
        self.levels[2] = (self.levels[2] & !timer_mask) | (timer_levels & timer_mask);
        if let Some((clock,mosi)) = serial {
            if self.pfcr & 0x10 == 0 {
                self.levels[3] = (self.levels[3] & !6) | u8::from(clock)<<1 | u8::from(mosi)<<2;
            } else {
                self.levels[3] = (self.levels[3] & !6) | u8::from(clock)<<2 | u8::from(mosi)<<1;
            }
        }
        if let Some(high) = miso { self.levels[3] = (self.levels[3] & !8) | u8::from(high)<<3; }
        SerialLevels { lcd_selected: self.levels[0] & 1 == 0,
            data: self.levels[0] & 2 != 0, eeprom_selected: self.levels[0] & 4 == 0,
            sensor_selected: self.levels[3] & 1 == 0,
            clock: self.levels[3] & 2 != 0, mosi: self.levels[3] & 4 != 0 }
    }
    pub fn serial_input(&self) -> bool {
        self.levels[3] & if self.pfcr & 0x10 == 0 {8} else {1} != 0
    }
    pub fn irq_levels(&self) -> [Option<bool>;2] {
        [match self.pfcr & 3 {
            0 if self.pmr[2] & 1 != 0 => Some(self.levels[4] & 1 != 0),
            1 => Some(self.levels[3] & 4 != 0), 2 => Some(self.levels[1] & 1 != 0), _ => None },
         match (self.pfcr >> 2) & 3 {
            0 if self.pmr[2] & 2 != 0 => Some(self.levels[4] & 2 != 0),
            1 => Some(self.levels[3] & 8 != 0), 2 => Some(self.levels[0] & 2 != 0), _ => None }]
    }
    pub fn piezo_levels(&self) -> (bool,bool) { (self.levels[2]&4!=0,self.levels[2]&8!=0) }
    pub fn battery_switch(&self) -> bool { self.levels[2] & 16 != 0 }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn button_pins_and_mux_are_not_firmware_flags() {
        let mut p=Gpio::default(); p.set_buttons(Buttons{left:true,center:false,right:true});
        p.write(0xffca,1).unwrap(); p.resolve(None,0,0,None);
        assert_eq!(p.read(0xffde)&0x15,0x14); assert_eq!(p.irq_levels()[0],Some(false));
    }
    #[test]
    fn chip_selects_have_independent_lifetimes() {
        let mut p=Gpio::default(); p.write(0xffe4,7).unwrap();p.write(0xffd4,4).unwrap();
        let s=p.resolve(Some((true,true)),0,0,Some(false));
        assert!(s.lcd_selected);assert!(!s.eeprom_selected);assert!(!s.data);assert!(!p.serial_input());
    }
}
