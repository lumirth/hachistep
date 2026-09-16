//! Incremental H8/300H normal-mode decoder. Only words already fetched by
//! the CPU are supplied. A prefix never peeks ahead into host backing memory.
//! Reference: ADE-602-053A tables 2-3 through 2-6; H8/38602R target restrictions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Byte,
    Word,
    Long,
}
impl Size {
    pub const fn bits(self) -> u32 {
        match self {
            Self::Byte => 8,
            Self::Word => 16,
            Self::Long => 32,
        }
    }
    pub const fn bytes(self) -> u16 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Long => 4,
        }
    }
    pub const fn mask(self) -> u32 {
        match self {
            Self::Byte => 255,
            Self::Word => 65535,
            Self::Long => u32::MAX,
        }
    }
    pub const fn sign(self) -> u32 {
        1u32 << (self.bits() - 1)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alu {
    Mov,
    Add,
    Addx,
    Sub,
    Subx,
    Cmp,
    And,
    Or,
    Xor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
    Not,
    Neg,
    Inc,
    Dec,
    Extu,
    Exts,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Shll,
    Shal,
    Shlr,
    Shar,
    Rotl,
    Rotr,
    Rotxl,
    Rotxr,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Reg(u8),
    Imm(u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    Absolute(u16),
    Indirect(u8),
    Displaced { reg: u8, offset: i32 },
    PostIncrement(u8),
    PreDecrement(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Reg(u8),
    Memory(Address),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bit {
    Set,
    Clear,
    Not,
    Test,
    Store(bool),
    Load(bool),
    And(bool),
    Or(bool),
    Xor(bool),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jump {
    Absolute(u16),
    Register(u8),
    Vector(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CcrOp {
    Load,
    And,
    Or,
    Xor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instruction {
    Nop,
    Sleep,
    Binary {
        op: Alu,
        size: Size,
        dst: u8,
        src: Source,
    },
    Unary {
        op: Unary,
        size: Size,
        dst: u8,
        amount: u32,
    },
    Shift {
        op: Shift,
        size: Size,
        dst: u8,
    },
    Quick {
        dst: u8,
        delta: i8,
    },
    Bit {
        op: Bit,
        bit: Source,
        target: Target,
    },
    Memory {
        size: Size,
        reg: u8,
        address: Address,
        store: bool,
        ccr: bool,
    },
    Branch {
        condition: u8,
        displacement: i16,
    },
    BranchSubroutine(i16),
    Jump {
        target: Jump,
        call: bool,
    },
    Return {
        exception: bool,
    },
    Trap(u8),
    Ccr {
        op: CcrOp,
        source: Source,
    },
    StoreCcr(u8),
    MulDiv {
        divide: bool,
        signed: bool,
        size: Size,
        src: u8,
        dst: u8,
    },
    Decimal {
        subtract: bool,
        reg: u8,
    },
    EepMov {
        word_count: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decode {
    NeedWord,
    Ready { instruction: Instruction, words: u8 },
    Invalid,
}

fn ready(instruction: Instruction, words: usize) -> Decode {
    Decode::Ready {
        instruction,
        words: words as u8,
    }
}
fn invalid() -> Decode {
    Decode::Invalid
}

pub fn decode(words: &[u16]) -> Decode {
    if words.is_empty() {
        return Decode::NeedWord;
    }
    let w = words[0];
    let hi = (w >> 8) as u8;
    let lo = w as u8;
    let s = lo >> 4;
    let d = lo & 15;
    use Instruction::*;
    use Size::*;
    if hi >= 0x80 {
        let op = match hi >> 4 {
            8 => Alu::Add,
            9 => Alu::Addx,
            10 => Alu::Cmp,
            11 => Alu::Subx,
            12 => Alu::Or,
            13 => Alu::Xor,
            14 => Alu::And,
            _ => Alu::Mov,
        };
        return ready(
            Binary {
                op,
                size: Byte,
                dst: hi & 15,
                src: Source::Imm(u32::from(lo)),
            },
            1,
        );
    }
    if (0x20..=0x3f).contains(&hi) {
        return ready(
            Memory {
                size: Byte,
                reg: hi & 15,
                address: Address::Absolute(0xff00 | u16::from(lo)),
                store: hi >= 0x30,
                ccr: false,
            },
            1,
        );
    }
    if (0x40..=0x4f).contains(&hi) {
        return ready(
            Branch {
                condition: hi & 15,
                displacement: lo as i8 as i16,
            },
            1,
        );
    }
    match hi {
        0x00 if lo == 0 => ready(Nop, 1),
        0x01 => match lo {
            0x80 => ready(Sleep, 1),
            0x00 | 0x40 => memory(words, 1, if lo == 0 { Long } else { Word }, lo == 0x40),
            0xc0 | 0xd0 | 0xf0 => {
                if words.len() < 2 {
                    return Decode::NeedWord;
                }
                let q = words[1];
                let h = (q >> 8) as u8;
                let src = ((q >> 4) & 15) as u8;
                let dst = (q & 15) as u8;
                if lo == 0xf0 && (0x64..=0x66).contains(&h) && src < 8 && dst < 8 {
                    return ready(
                        Binary {
                            op: match h {
                                0x64 => Alu::Or,
                                0x65 => Alu::Xor,
                                _ => Alu::And,
                            },
                            size: Long,
                            dst,
                            src: Source::Reg(src),
                        },
                        2,
                    );
                }
                if (lo == 0xc0 && (h == 0x50 || h == 0x52))
                    || (lo == 0xd0 && (h == 0x51 || h == 0x53))
                {
                    if h & 2 != 0 && dst > 7 {
                        return invalid();
                    }
                    return ready(
                        MulDiv {
                            divide: lo == 0xd0,
                            signed: true,
                            size: if h & 2 == 0 { Byte } else { Word },
                            src,
                            dst,
                        },
                        2,
                    );
                }
                invalid()
            }
            _ => invalid(),
        },
        0x02 if s == 0 => ready(StoreCcr(d), 1),
        0x03 if s == 0 => ready(
            Ccr {
                op: CcrOp::Load,
                source: Source::Reg(d),
            },
            1,
        ),
        0x04..=0x07 => ready(
            Ccr {
                op: match hi {
                    4 => CcrOp::Or,
                    5 => CcrOp::Xor,
                    6 => CcrOp::And,
                    _ => CcrOp::Load,
                },
                source: Source::Imm(u32::from(lo)),
            },
            1,
        ),
        0x08 | 0x09 | 0x0c | 0x0d | 0x0e | 0x14 | 0x15 | 0x16 | 0x18 | 0x19 | 0x1c | 0x1d
        | 0x1e | 0x64 | 0x65 | 0x66 => {
            let (op, size) = match hi {
                8 => (Alu::Add, Byte),
                9 => (Alu::Add, Word),
                12 => (Alu::Mov, Byte),
                13 => (Alu::Mov, Word),
                14 => (Alu::Addx, Byte),
                0x14 => (Alu::Or, Byte),
                0x15 => (Alu::Xor, Byte),
                0x16 => (Alu::And, Byte),
                0x18 => (Alu::Sub, Byte),
                0x19 => (Alu::Sub, Word),
                0x1c => (Alu::Cmp, Byte),
                0x1d => (Alu::Cmp, Word),
                0x1e => (Alu::Subx, Byte),
                0x64 => (Alu::Or, Word),
                0x65 => (Alu::Xor, Word),
                _ => (Alu::And, Word),
            };
            ready(
                Binary {
                    op,
                    size,
                    dst: d,
                    src: Source::Reg(s),
                },
                1,
            )
        }
        0x0a | 0x1a => {
            if s == 0 {
                return ready(
                    Unary {
                        op: if hi == 0x0a {
                            super::decode::Unary::Inc
                        } else {
                            super::decode::Unary::Dec
                        },
                        size: Byte,
                        dst: d,
                        amount: 1,
                    },
                    1,
                );
            }
            if s >= 8 && d < 8 {
                return ready(
                    Binary {
                        op: if hi == 0x0a { Alu::Add } else { Alu::Sub },
                        size: Long,
                        dst: d,
                        src: Source::Reg(s & 7),
                    },
                    1,
                );
            }
            invalid()
        }
        0x0b | 0x1b => {
            let subtract = hi == 0x1b;
            if matches!(s, 0 | 8 | 9) && d < 8 {
                let amount = match s {
                    0 => 1,
                    8 => 2,
                    _ => 4,
                };
                return ready(
                    Quick {
                        dst: d,
                        delta: if subtract { -amount } else { amount },
                    },
                    1,
                );
            }
            if matches!(s, 5 | 7 | 13 | 15) && (s & 2 == 0 || d < 8) {
                return ready(
                    Unary {
                        op: if subtract {
                            super::decode::Unary::Dec
                        } else {
                            super::decode::Unary::Inc
                        },
                        size: if s & 2 == 0 { Word } else { Long },
                        dst: d,
                        amount: if s & 8 == 0 { 1 } else { 2 },
                    },
                    1,
                );
            }
            invalid()
        }
        0x0f | 0x1f => {
            if s == 0 {
                return ready(
                    Decimal {
                        subtract: hi == 0x1f,
                        reg: d,
                    },
                    1,
                );
            }
            if s >= 8 && d < 8 {
                return ready(
                    Binary {
                        op: if hi == 0x0f { Alu::Mov } else { Alu::Cmp },
                        size: Long,
                        dst: d,
                        src: Source::Reg(s & 7),
                    },
                    1,
                );
            }
            invalid()
        }
        0x10..=0x13 => {
            let size = match s & 7 {
                0 => Byte,
                1 => Word,
                3 if d < 8 => Long,
                _ => return invalid(),
            };
            let op = match (hi, s & 8 != 0) {
                (0x10, false) => super::decode::Shift::Shll,
                (0x10, true) => super::decode::Shift::Shal,
                (0x11, false) => super::decode::Shift::Shlr,
                (0x11, true) => super::decode::Shift::Shar,
                (0x12, false) => super::decode::Shift::Rotxl,
                (0x12, true) => super::decode::Shift::Rotl,
                (0x13, false) => super::decode::Shift::Rotxr,
                _ => super::decode::Shift::Rotr,
            };
            ready(Shift { op, size, dst: d }, 1)
        }
        0x17 => {
            let (op, size) = match s {
                0 => (super::decode::Unary::Not, Byte),
                1 => (super::decode::Unary::Not, Word),
                3 => (super::decode::Unary::Not, Long),
                8 => (super::decode::Unary::Neg, Byte),
                9 => (super::decode::Unary::Neg, Word),
                11 => (super::decode::Unary::Neg, Long),
                5 => (super::decode::Unary::Extu, Word),
                7 => (super::decode::Unary::Extu, Long),
                13 => (super::decode::Unary::Exts, Word),
                15 => (super::decode::Unary::Exts, Long),
                _ => return invalid(),
            };
            if size == Long && d >= 8 {
                return invalid();
            }
            ready(
                Unary {
                    op,
                    size,
                    dst: d,
                    amount: 0,
                },
                1,
            )
        }
        0x50..=0x53 => {
            let size = if hi & 2 == 0 { Byte } else { Word };
            if size == Word && d > 7 {
                return invalid();
            }
            ready(
                MulDiv {
                    divide: hi & 1 != 0,
                    signed: false,
                    size,
                    src: s,
                    dst: d,
                },
                1,
            )
        }
        0x54 if lo == 0x70 => ready(Return { exception: false }, 1),
        0x55 => ready(BranchSubroutine(lo as i8 as i16), 1),
        0x56 if lo == 0x70 => ready(Return { exception: true }, 1),
        0x57 if d == 0 && s < 4 => ready(Trap(s), 1),
        0x58 if d == 0 => {
            if words.len() < 2 {
                return Decode::NeedWord;
            }
            ready(
                Branch {
                    condition: s,
                    displacement: words[1] as i16,
                },
                2,
            )
        }
        0x59 | 0x5d if d == 0 && s < 8 => ready(
            Jump {
                target: super::decode::Jump::Register(s),
                call: hi == 0x5d,
            },
            1,
        ),
        0x5a | 0x5e => {
            if words.len() < 2 {
                return Decode::NeedWord;
            }
            ready(
                Jump {
                    target: super::decode::Jump::Absolute(words[1]),
                    call: hi == 0x5e,
                },
                2,
            )
        }
        0x5b | 0x5f => ready(
            Jump {
                target: super::decode::Jump::Vector(lo),
                call: hi == 0x5f,
            },
            1,
        ),
        0x5c if lo == 0 => {
            if words.len() < 2 {
                return Decode::NeedWord;
            }
            ready(BranchSubroutine(words[1] as i16), 2)
        }
        0x60..=0x63 | 0x67 | 0x70..=0x77 => match bit_operation(hi, s) {
            Some((op, bit)) => ready(
                Bit {
                    op,
                    bit,
                    target: Target::Reg(d),
                },
                1,
            ),
            None => invalid(),
        },
        0x68..=0x6f | 0x78 => memory(words, 0, if hi & 1 == 0 { Byte } else { Word }, false),
        0x79 | 0x7a => {
            let length = if hi == 0x79 { 2 } else { 3 };
            if s > 6 || (hi == 0x7a && d >= 8) {
                return invalid();
            }
            if words.len() < length {
                return Decode::NeedWord;
            }
            let value = if hi == 0x79 {
                u32::from(words[1])
            } else {
                u32::from(words[1]) << 16 | u32::from(words[2])
            };
            let op = match s {
                0 => Alu::Mov,
                1 => Alu::Add,
                2 => Alu::Cmp,
                3 => Alu::Sub,
                4 => Alu::Or,
                5 => Alu::Xor,
                _ => Alu::And,
            };
            ready(
                Binary {
                    op,
                    size: if hi == 0x79 { Word } else { Long },
                    dst: d,
                    src: Source::Imm(value),
                },
                length,
            )
        }
        0x7b if lo == 0x5c || lo == 0xd4 => {
            if words.len() < 2 {
                return Decode::NeedWord;
            }
            if words[1] != 0x598f {
                return invalid();
            }
            ready(
                EepMov {
                    word_count: lo == 0xd4,
                },
                2,
            )
        }
        0x7c..=0x7f => {
            if (hi == 0x7c || hi == 0x7d) && (s > 7 || d != 0) {
                return invalid();
            }
            if words.len() < 2 {
                return Decode::NeedWord;
            }
            let q = words[1];
            if q & 15 != 0 {
                return invalid();
            }
            let Some((op, bit)) = bit_operation((q >> 8) as u8, ((q >> 4) & 15) as u8) else {
                return invalid();
            };
            let writes = matches!(
                op,
                super::decode::Bit::Set
                    | super::decode::Bit::Clear
                    | super::decode::Bit::Not
                    | super::decode::Bit::Store(_)
            );
            if writes != (hi & 1 != 0) {
                return invalid();
            }
            let addr = if hi < 0x7e {
                Address::Indirect(s)
            } else {
                Address::Absolute(0xff00 | u16::from(lo))
            };
            ready(
                Bit {
                    op,
                    bit,
                    target: Target::Memory(addr),
                },
                2,
            )
        }
        _ => invalid(),
    }
}
fn bit_operation(code: u8, selector: u8) -> Option<(Bit, Source)> {
    let imm = Source::Imm(u32::from(selector & 7));
    let inv = selector & 8 != 0;
    Some(match code {
        0x60 => (Bit::Set, Source::Reg(selector)),
        0x61 => (Bit::Not, Source::Reg(selector)),
        0x62 => (Bit::Clear, Source::Reg(selector)),
        0x63 => (Bit::Test, Source::Reg(selector)),
        0x67 => (Bit::Store(inv), imm),
        0x70 if !inv => (Bit::Set, imm),
        0x71 if !inv => (Bit::Not, imm),
        0x72 if !inv => (Bit::Clear, imm),
        0x73 if !inv => (Bit::Test, imm),
        0x74 => (Bit::Or(inv), imm),
        0x75 => (Bit::Xor(inv), imm),
        0x76 => (Bit::And(inv), imm),
        0x77 => (Bit::Load(inv), imm),
        _ => return None,
    })
}
fn memory(words: &[u16], prefix: usize, selected_size: Size, ccr: bool) -> Decode {
    if words.len() <= prefix {
        return Decode::NeedWord;
    }
    let w = words[prefix];
    let code = (w >> 8) as u8;
    let sel = ((w >> 4) & 15) as u8;
    let reg = (w & 15) as u8;
    let store = sel & 8 != 0;
    let size = if prefix != 0 {
        selected_size
    } else if code & 1 == 0 {
        Size::Byte
    } else {
        Size::Word
    };
    if (size == Size::Long && reg >= 8) || (ccr && reg != 0) {
        return Decode::Invalid;
    }
    let (address, used, actual_store, actual_reg, actual_size) = match code {
        0x68 | 0x69 => (Address::Indirect(sel & 7), prefix + 1, store, reg, size),
        0x6c | 0x6d => (
            if store {
                Address::PreDecrement(sel & 7)
            } else {
                Address::PostIncrement(sel & 7)
            },
            prefix + 1,
            store,
            reg,
            size,
        ),
        0x6e | 0x6f => {
            if words.len() < prefix + 2 {
                return Decode::NeedWord;
            }
            (
                Address::Displaced {
                    reg: sel & 7,
                    offset: words[prefix + 1] as i16 as i32,
                },
                prefix + 2,
                store,
                reg,
                size,
            )
        }
        0x6a | 0x6b => {
            let long_addr = sel & 2 != 0;
            if !matches!(sel, 0 | 2 | 8 | 10) {
                return Decode::Invalid;
            }
            let used = prefix + if long_addr { 3 } else { 2 };
            if words.len() < used {
                return Decode::NeedWord;
            }
            if long_addr && words[prefix + 1] & 0xff00 != 0 {
                return Decode::Invalid;
            }
            (Address::Absolute(words[used - 1]), used, store, reg, size)
        }
        0x78 => {
            // The high address-register selector bit is fixed zero, not an
            // additional store bit. Store direction belongs to word two.
            if reg != 0 || sel >= 8 {
                return Decode::Invalid;
            }
            if words.len() < prefix + 2 {
                return Decode::NeedWord;
            }
            let q = words[prefix + 1];
            let h = (q >> 8) as u8;
            let form = ((q >> 4) & 15) as u8;
            if !matches!(h, 0x6a | 0x6b) || !matches!(form, 2 | 10) || (prefix != 0 && h != 0x6b) {
                return Decode::Invalid;
            }
            let actual_size = if prefix != 0 {
                selected_size
            } else if h == 0x6a {
                Size::Byte
            } else {
                Size::Word
            };
            let actual_reg = (q & 15) as u8;
            if (actual_size == Size::Long && actual_reg >= 8) || (ccr && actual_reg != 0) {
                return Decode::Invalid;
            }
            if words.len() < prefix + 4 {
                return Decode::NeedWord;
            }
            let raw = u32::from(words[prefix + 2]) << 16 | u32::from(words[prefix + 3]);
            if raw >> 24 != 0 {
                return Decode::Invalid;
            }
            let offset = ((raw << 8) as i32) >> 8;
            (
                Address::Displaced {
                    reg: sel & 7,
                    offset,
                },
                prefix + 4,
                form & 8 != 0,
                actual_reg,
                actual_size,
            )
        }
        _ => return Decode::Invalid,
    };
    if prefix != 0 && !matches!(code, 0x69 | 0x6b | 0x6d | 0x6f | 0x78) {
        return Decode::Invalid;
    }
    ready(
        Instruction::Memory {
            size: actual_size,
            reg: actual_reg,
            address,
            store: actual_store,
            ccr,
        },
        used,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_forms_and_extension_boundaries() {
        assert_eq!(decode(&[0x7907]), Decode::NeedWord);
        assert_eq!(
            decode(&[0x7907, 0xff80]),
            ready(
                Instruction::Binary {
                    op: Alu::Mov,
                    size: Size::Word,
                    dst: 7,
                    src: Source::Imm(0xff80)
                },
                2
            )
        );
        assert_eq!(
            decode(&[0x0100, 0x6df4]),
            ready(
                Instruction::Memory {
                    size: Size::Long,
                    reg: 4,
                    address: Address::PreDecrement(7),
                    store: true,
                    ccr: false
                },
                2
            )
        );
        assert_eq!(
            decode(&[0x7eb1, 0x7700]),
            ready(
                Instruction::Bit {
                    op: Bit::Load(false),
                    bit: Source::Imm(0),
                    target: Target::Memory(Address::Absolute(0xffb1))
                },
                2
            )
        );
        assert_eq!(
            decode(&[0x7b5c, 0x598f]),
            ready(Instruction::EepMov { word_count: false }, 2)
        );
        assert_eq!(decode(&[0x6a40, 0x1234]), Decode::Invalid); // MOVFPE absent on this target.
    }
    #[test]
    fn all_first_words_classify_without_panicking() {
        for w in 0..=u16::MAX {
            let _ = decode(&[w]);
        }
    }
}
