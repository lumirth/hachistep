//! Exact chronological bit sequences for serial shifts and electrical drives.
//! Lane zero is the next edge. A one-edge sequence uses these same operations.
use crate::signals::Drive;

pub(crate) fn mask(count: u8) -> u16 {
    ((1u32 << count) - 1) as u16
}

#[derive(Clone, Copy)]
pub(crate) struct Bits {
    pub value: u8,
    pub count: u8,
}
impl Bits {
    pub fn one(high: bool) -> Self {
        Self {
            value: u8::from(high),
            count: 1,
        }
    }
    pub fn output(byte: u8, bit: u8, count: u8) -> Self {
        Self {
            value: if bit < 8 {
                byte.reverse_bits() >> bit
            } else {
                0
            },
            count,
        }
    }
    pub fn samples(levels: u16, positions: u16) -> Self {
        let count = positions.count_ones() as u8;
        let even = mask(2 * count) & 0x5555;
        if positions == even || positions == even << 1 {
            let mut value = if positions == even {
                levels
            } else {
                levels >> 1
            } & even;
            value = (value | value >> 1) & 0x3333;
            value = (value | value >> 2) & 0x0f0f;
            value = (value | value >> 4) & 0x00ff;
            return Self {
                value: value as u8,
                count,
            };
        }
        let mut value = 0;
        let mut remaining = positions;
        let mut bit = 0;
        while remaining != 0 {
            value |= u8::from(levels & (1 << remaining.trailing_zeros()) != 0) << bit;
            remaining &= remaining - 1;
            bit += 1;
        }
        Self { value, count }
    }
    pub fn append(self, value: &mut u8, count: &mut u8) {
        if self.count == 0 {
            return;
        }
        let incoming = self.value.reverse_bits() >> (8 - self.count);
        *value = ((u16::from(*value) << self.count) | u16::from(incoming)) as u8;
        *count += self.count;
    }
}

pub(crate) fn spread(byte: u8) -> u16 {
    let mut value = u16::from(byte);
    value = (value | value << 4) & 0x0f0f;
    value = (value | value << 2) & 0x3333;
    (value | value << 1) & 0x5555
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Drives {
    pub low: u16,
    pub high: u16,
}
impl Drives {
    pub fn wired(self, other: Self) -> Self {
        let low = self.low | other.low;
        Self {
            low,
            high: (self.high | other.high) & !low,
        }
    }
    pub fn constant(drive: Drive, lanes: u16) -> Self {
        match drive {
            Drive::Low => Self {
                low: lanes,
                high: 0,
            },
            Drive::High => Self {
                low: 0,
                high: lanes,
            },
            Drive::Floating => Self::default(),
        }
    }
    pub fn levels(high: u16, lanes: u16, open_drain: bool) -> Self {
        Self {
            low: !high & lanes,
            high: if open_drain { 0 } else { high & lanes },
        }
    }
    pub fn at(self, index: u8) -> Drive {
        let bit = 1 << index;
        if self.low & bit != 0 {
            Drive::Low
        } else if self.high & bit != 0 {
            Drive::High
        } else {
            Drive::Floating
        }
    }
    pub fn before(self, initial: Drive, lanes: u16) -> Self {
        let first = Self::constant(initial, 1);
        Self {
            low: (self.low << 1 | first.low) & lanes,
            high: (self.high << 1 | first.high) & lanes,
        }
    }
    /// Already-latched output bits launch on alternating edges. No parser or
    /// read capture occurs here; the owner bounds the sequence before those effects.
    pub fn launches(byte: u8, bit: u8, events: u16, initial: Drive, lanes: u16) -> Self {
        if events == 0 {
            return Self::constant(initial, lanes);
        }
        let first = events.trailing_zeros() as u8;
        let bits = Bits::output(byte, bit, events.count_ones() as u8).value;
        let repeated = spread(bits);
        let high = (repeated | repeated << 1) << first;
        let available = (2 * (8 - bit.min(8)) + first).min(16);
        let driven = mask(available) & !mask(first) & lanes;
        let leading = Self::constant(initial, mask(first));
        Self {
            low: (!high & driven) | leading.low,
            high: (high & driven) | leading.high,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sampled_lanes_preserve_chronological_bit_order() {
        for count in 0..=8 {
            for odd in [false, true] {
                let positions =
                    (0..count).fold(0, |mask, bit| mask | (1 << (2 * bit + u8::from(odd))));
                for levels in 0..=u16::MAX {
                    let expected = (0..count).fold(0u8, |value, bit| {
                        value | (((levels >> (2 * bit + u8::from(odd))) & 1) as u8) << bit
                    });
                    let sampled = Bits::samples(levels, positions);
                    assert_eq!((sampled.value, sampled.count), (expected, count));
                }
            }
        }
        let sample = Bits::samples(0b1000_0101, 0b1100_1001);
        assert_eq!((sample.value, sample.count), (0b1001, 4));
    }
    #[test]
    fn latched_output_planes_include_release_after_the_last_bit() {
        for byte in 0..=u8::MAX {
            for bit in 0..=8 {
                for first in [0, 1] {
                    for count in 1..=16 {
                        let events = (if first == 0 { 0x5555 } else { 0xaaaa }) & mask(count);
                        for initial in [Drive::Low, Drive::High, Drive::Floating] {
                            let plane = Drives::launches(byte, bit, events, initial, mask(count));
                            let mut next = bit;
                            let mut driver = initial;
                            for edge in 0..count {
                                if events & (1 << edge) != 0 {
                                    driver = if next < 8 {
                                        if byte & (0x80 >> next) != 0 {
                                            Drive::High
                                        } else {
                                            Drive::Low
                                        }
                                    } else {
                                        Drive::Floating
                                    };
                                    next = next.saturating_add(1).min(8);
                                }
                                assert_eq!(plane.at(edge), driver);
                            }
                        }
                    }
                }
            }
        }
    }
}
