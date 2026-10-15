//! H8 arithmetic with explicit guest widths, shifts and flag updates.
//! Each instruction preserves the fields it does not own.
use super::decode::{Alu, Bit, Shift, Size, Unary};
pub const C: u8 = 1;
pub const V: u8 = 2;
pub const Z: u8 = 4;
pub const N: u8 = 8;
pub const H: u8 = 0x20;
pub const I: u8 = 0x80;

pub fn nz(value: u32, size: Size) -> u8 {
    let value = value & size.mask();
    (if value == 0 { Z } else { 0 }) | (if value & size.sign() != 0 { N } else { 0 })
}
pub fn logical(value: u32, size: Size, ccr: u8) -> u8 {
    (ccr & !(N | Z | V)) | nz(value, size)
}
pub fn binary(op: Alu, size: Size, a: u32, b: u32, ccr: u8) -> (u32, u8) {
    let mask = size.mask();
    let a = a & mask;
    let b = b & mask;
    match op {
        Alu::Mov => (b, logical(b, size, ccr)),
        Alu::And | Alu::Or | Alu::Xor => {
            let r = match op {
                Alu::And => a & b,
                Alu::Or => a | b,
                _ => a ^ b,
            };
            (r, logical(r, size, ccr))
        }
        Alu::Add | Alu::Addx | Alu::Sub | Alu::Subx | Alu::Cmp => {
            let extended = matches!(op, Alu::Addx | Alu::Subx);
            let carry_in = u64::from(extended && ccr & C != 0);
            let subtract = matches!(op, Alu::Sub | Alu::Subx | Alu::Cmp);
            let rhs = u64::from(b) + carry_in;
            let raw = if subtract {
                u64::from(a).wrapping_sub(rhs)
            } else {
                u64::from(a) + rhs
            };
            let r = raw as u32 & mask;
            let carry = if subtract {
                u64::from(a) < rhs
            } else {
                raw > u64::from(mask)
            };
            let sign = size.sign();
            let overflow = if subtract {
                ((a ^ b) & (a ^ r) & sign) != 0
            } else {
                (!(a ^ b) & (a ^ r) & sign) != 0
            };
            let halfmask = (1u64 << (size.bits() - 4)) - 1;
            let half = if subtract {
                (u64::from(a) & halfmask) < (u64::from(b) & halfmask) + carry_in
            } else {
                (u64::from(a) & halfmask) + (u64::from(b) & halfmask) + carry_in > halfmask
            };
            let mut flags = nz(r, size);
            if extended && ccr & Z == 0 {
                flags &= !Z;
            }
            flags |= if carry { C } else { 0 };
            flags |= if half { H } else { 0 };
            flags |= if overflow { V } else { 0 };
            (r, (ccr & !(H | N | Z | V | C)) | flags)
        }
    }
}
pub fn unary(op: Unary, size: Size, a: u32, amount: u32, ccr: u8) -> (u32, u8) {
    let a = a & size.mask();
    match op {
        Unary::Not => {
            let r = !a & size.mask();
            (r, logical(r, size, ccr))
        }
        Unary::Neg => binary(Alu::Sub, size, 0, a, ccr),
        Unary::Inc | Unary::Dec => {
            let (r, f) = binary(
                if op == Unary::Inc { Alu::Add } else { Alu::Sub },
                size,
                a,
                amount,
                ccr,
            );
            (r, (ccr & !(N | Z | V)) | (f & (N | Z | V)))
        }
        Unary::Extu => {
            let r = a & if size == Size::Word { 0xff } else { 0xffff };
            (r, logical(r, size, ccr))
        }
        Unary::Exts => {
            let r = if size == Size::Word {
                (a as u8 as i8 as i32) as u32 & 0xffff
            } else {
                (a as u16 as i16 as i32) as u32
            };
            (r, logical(r, size, ccr))
        }
    }
}
pub fn shift(op: Shift, size: Size, a: u32, ccr: u8) -> (u32, u8) {
    let a = a & size.mask();
    let sign = size.sign();
    let old_c = u32::from(ccr & C != 0);
    let left = matches!(op, Shift::Shll | Shift::Shal | Shift::Rotl | Shift::Rotxl);
    let carry = if left { a & sign != 0 } else { a & 1 != 0 };
    let r = match op {
        Shift::Shll | Shift::Shal => a.wrapping_shl(1),
        Shift::Shlr => a >> 1,
        Shift::Shar => (a >> 1) | (a & sign),
        Shift::Rotl => a.wrapping_shl(1) | u32::from(carry),
        Shift::Rotr => (a >> 1) | if carry { sign } else { 0 },
        Shift::Rotxl => a.wrapping_shl(1) | old_c,
        Shift::Rotxr => (a >> 1) | if old_c != 0 { sign } else { 0 },
    } & size.mask();
    let overflow = op == Shift::Shal && ((a ^ r) & sign != 0);
    let f = (ccr & !(N | Z | V | C))
        | nz(r, size)
        | if carry { C } else { 0 }
        | if overflow { V } else { 0 };
    (r, f)
}
#[inline(always)]
pub fn condition(code: u8, flags: u8) -> bool {
    let (c, z, n, v) = (
        flags & C != 0,
        flags & Z != 0,
        flags & N != 0,
        flags & V != 0,
    );
    match code & 15 {
        0 => true,
        1 => false,
        2 => !c && !z,
        3 => c || z,
        4 => !c,
        5 => c,
        6 => !z,
        7 => z,
        8 => !v,
        9 => v,
        10 => !n,
        11 => n,
        12 => n == v,
        13 => n != v,
        14 => !z && n == v,
        _ => z || n != v,
    }
}

#[inline(always)]
pub fn bit(ccr: &mut u8, op: Bit, bit: u8, value: u8) -> (u8, bool) {
    let mask = 1u8 << (bit & 7);
    let selected = value & mask != 0;
    let old = *ccr & C != 0;
    match op {
        Bit::Set => (value | mask, true),
        Bit::Clear => (value & !mask, true),
        Bit::Not => (value ^ mask, true),
        Bit::Test => {
            *ccr = (*ccr & !Z) | if selected { 0 } else { Z };
            (value, false)
        }
        Bit::Store(inv) => (
            if old ^ inv {
                value | mask
            } else {
                value & !mask
            },
            true,
        ),
        Bit::Load(inv) | Bit::And(inv) | Bit::Or(inv) | Bit::Xor(inv) => {
            let input = selected ^ inv;
            let result = match op {
                Bit::Load(_) => input,
                Bit::And(_) => old && input,
                Bit::Or(_) => old || input,
                _ => old ^ input,
            };
            *ccr = (*ccr & !C) | if result { C } else { 0 };
            (value, false)
        }
    }
}
