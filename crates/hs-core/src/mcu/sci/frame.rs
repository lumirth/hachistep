//! Character interpretation is latched at TSR load or receive start. Clock
//! selection remains live independently of that interpretation.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Format {
    pub synchronous: bool,
    pub data: u8,
    pub parity: bool,
    pub odd: bool,
    pub stops: u8,
    pub half_bit: u64,
}
impl Format {
    pub fn new(smr: u8, semr: u8) -> Self {
        let synchronous = smr & 0x80 != 0;
        let short = smr & 4 != 0;
        Self {
            synchronous,
            data: if synchronous {
                8
            } else if short {
                5
            } else if smr & 0x40 != 0 {
                7
            } else {
                8
            },
            parity: !synchronous && smr & 0x20 != 0 && (!short || smr & 0x40 != 0),
            odd: smr & 0x10 != 0,
            stops: 1 + u8::from(smr & 8 != 0),
            half_bit: if semr & 8 != 0 { 16 } else { 32 },
        }
    }
    pub fn stop(self) -> u8 {
        1 + self.data + u8::from(self.parity)
    }
    pub fn cells(self) -> u8 {
        self.stop() + self.stops
    }
    pub fn word(self, value: u8) -> u16 {
        if self.synchronous {
            return u16::from(value);
        }
        let value = value & ((1u16 << self.data) - 1) as u8;
        let mut word = u16::from(value) << 1;
        if self.parity {
            word |= u16::from((value.count_ones() & 1 != 0) ^ self.odd) << (self.data + 1);
        }
        word | (((1 << self.stops) - 1) << self.stop())
    }
}

impl Format {
    pub(super) fn validate(self) -> Result<(), crate::Error> {
        crate::state::require(
            matches!(self.data, 5 | 7 | 8)
                && matches!(self.stops, 1 | 2)
                && matches!(self.half_bit, 16 | 32)
                && (!self.synchronous || (self.data == 8 && !self.parity)),
            "invalid serial frame format",
        )
    }
}
