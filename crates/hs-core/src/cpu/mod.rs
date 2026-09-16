//! A single resumable execution engine. `Action` describes the next physical
//! access or internal wait; `complete` commits only that action. There is no
//! atomic-instruction executor or rollback path.
pub mod alu;
pub mod decode;
use crate::error::Error;
use alu::{C, H, I, N, Z};
use decode::{Address, Alu, Bit, CcrOp, Decode, Instruction, Jump, Size, Source, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
}
impl Width {
    pub const fn bytes(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Read {
        address: u16,
        width: Width,
        fetch: bool,
    },
    Write {
        address: u16,
        width: Width,
        value: u16,
        mov_byte: bool,
    },
    Idle(u32),
    Sleep,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registers {
    pub er: [u32; 8],
    pub pc: u16,
    pub ccr: u8,
}
impl Registers {
    pub fn read(&self, size: Size, field: u8) -> u32 {
        let r = self.er[usize::from(field & 7)];
        match size {
            Size::Byte => {
                if field < 8 {
                    r >> 8 & 255
                } else {
                    r & 255
                }
            }
            Size::Word => {
                if field < 8 {
                    r & 65535
                } else {
                    r >> 16
                }
            }
            Size::Long => r,
        }
    }
    pub fn write(&mut self, size: Size, field: u8, value: u32) {
        let r = &mut self.er[usize::from(field & 7)];
        match size {
            Size::Byte => {
                if field < 8 {
                    *r = (*r & !0xff00) | (value & 255) << 8;
                } else {
                    *r = (*r & !255) | (value & 255);
                }
            }
            Size::Word => {
                if field < 8 {
                    *r = (*r & !65535) | (value & 65535);
                } else {
                    *r = (*r & 65535) | (value & 65535) << 16;
                }
            }
            Size::Long => *r = value,
        }
    }
    pub fn sp(&self) -> u16 {
        self.er[7] as u16
    }
    fn move_sp(&mut self, delta: i32) {
        self.er[7] = self.er[7].wrapping_add(delta as u32);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Transfer {
    address: u16,
    size: Size,
    register: u8,
    store: bool,
    ccr: bool,
    value: u32,
    done: u8,
    post: Option<(u8, u32)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Boundary,
    Fetch,
    Ready(Instruction),
    Memory(Transfer),
    BitRead {
        address: u16,
        op: Bit,
        bit: u8,
    },
    BitWrite {
        address: u16,
        value: u8,
    },
    IndirectJump {
        address: u16,
        call: bool,
    },
    Call {
        address: u16,
        target: u16,
        return_pc: u16,
    },
    BranchWait {
        target: u16,
    },
    ReturnPc,
    ReturnCcr,
    ReturnExceptionPc {
        ccr: u8,
    },
    ExceptionPc {
        vector: u8,
        pc: u16,
        ccr: u8,
    },
    ExceptionCcr {
        vector: u8,
        ccr: u8,
    },
    ExceptionVector {
        vector: u8,
    },
    MulDiv {
        size: Size,
        dst: u8,
        value: u32,
        flags: u8,
        states: u32,
    },
    Copy {
        word_count: bool,
        stage: u8,
        value: u8,
    },
    Sleeping,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub registers: Registers,
    phase: Phase,
    pub(crate) words: [u16; 5],
    pub(crate) word_count: u8,
    pub(crate) instruction_pc: u16,
    pub retired: u64,
    pub interrupt_entries: u64,
    interrupt_delay: u8,
}
impl Cpu {
    /// The caller obtains the reset vector through its MCU memory authority.
    /// Zero general-register startup is a deterministic witness, not a claim
    /// about physical power-on SRAM/register values.
    pub fn new(reset_vector: u16) -> Self {
        Self {
            registers: Registers {
                er: [0; 8],
                pc: reset_vector & !1,
                ccr: I,
            },
            phase: Phase::Boundary,
            words: [0; 5],
            word_count: 0,
            instruction_pc: reset_vector & !1,
            retired: 0,
            interrupt_entries: 0,
            interrupt_delay: 0,
        }
    }
    pub fn sleeping(&self) -> bool {
        self.phase == Phase::Sleeping
    }
    pub fn boundary(&self) -> bool {
        matches!(self.phase, Phase::Boundary | Phase::Sleeping)
    }
    pub fn instruction_pc(&self) -> u16 {
        self.instruction_pc
    }
    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Boundary => "boundary",
            Phase::Fetch => "fetch",
            Phase::Ready(_) => "decoded",
            Phase::Memory(_) => "memory",
            Phase::BitRead { .. } => "bit-read",
            Phase::BitWrite { .. } => "bit-write",
            Phase::IndirectJump { .. } => "indirect-jump",
            Phase::Call { .. } => "call-stack",
            Phase::BranchWait { .. } => "branch-wait",
            Phase::ReturnPc => "return-pc",
            Phase::ReturnCcr => "return-ccr",
            Phase::ReturnExceptionPc { .. } => "return-exception-pc",
            Phase::ExceptionPc { .. } => "exception-save-pc",
            Phase::ExceptionCcr { .. } => "exception-save-ccr",
            Phase::ExceptionVector { .. } => "exception-vector",
            Phase::MulDiv { .. } => "multiply-divide",
            Phase::Copy { .. } => "eepmov",
            Phase::Sleeping => "sleep",
        }
    }
    fn source(&self, size: Size, source: Source) -> u32 {
        match source {
            Source::Reg(r) => self.registers.read(size, r),
            Source::Imm(v) => v & size.mask(),
        }
    }
    fn finish(&mut self) {
        self.retired = self.retired.wrapping_add(1);
        self.phase = Phase::Boundary;
    }
    fn target_address(&mut self, address: Address, size: Size) -> (u16, Option<(u8, u32)>) {
        match address {
            Address::Absolute(v) => (v, None),
            Address::Indirect(r) => (self.registers.er[usize::from(r)] as u16, None),
            Address::Displaced { reg, offset } => (
                self.registers.er[usize::from(reg)].wrapping_add(offset as u32) as u16,
                None,
            ),
            Address::PostIncrement(r) => {
                let old = self.registers.er[usize::from(r)];
                (
                    old as u16,
                    Some((r, old.wrapping_add(u32::from(size.bytes())))),
                )
            }
            Address::PreDecrement(r) => {
                let value = self.registers.er[usize::from(r)].wrapping_sub(u32::from(size.bytes()));
                self.registers.er[usize::from(r)] = value;
                (value as u16, None)
            }
        }
    }
    pub(crate) fn direct_transition(&mut self) -> Result<(), Error> {
        if !self.sleeping() {
            return Err(Error::Internal("direct transition outside SLEEP"));
        }
        self.enter_exception(13);
        Ok(())
    }
    fn enter_exception(&mut self, vector: u8) {
        let pc = self.registers.pc;
        let ccr = self.registers.ccr;
        self.registers.ccr |= I;
        self.interrupt_entries = self.interrupt_entries.wrapping_add(1);
        self.registers.move_sp(-2);
        self.phase = Phase::ExceptionPc { vector, pc, ccr };
    }
    /// Produce the next action. A request is stable until `complete` is called.
    /// `interrupt` is a controller-selected vector, not an already-cleared flag.
    pub fn next(&mut self, interrupt: Option<u8>) -> Result<Action, Error> {
        loop {
            match self.phase {
                Phase::Boundary | Phase::Sleeping => {
                    if self.interrupt_delay == 0 {
                        if let Some(v) = interrupt {
                            if v == 7 || self.registers.ccr & I == 0 {
                                self.enter_exception(v);
                                continue;
                            }
                        }
                    }
                    if self.phase == Phase::Sleeping {
                        return Ok(Action::Sleep);
                    }
                    self.interrupt_delay = self.interrupt_delay.saturating_sub(1);
                    self.instruction_pc = self.registers.pc;
                    self.word_count = 0;
                    self.words = [0; 5];
                    self.phase = Phase::Fetch;
                }
                Phase::Fetch => {
                    return Ok(Action::Read {
                        address: self.registers.pc & !1,
                        width: Width::Word,
                        fetch: true,
                    })
                }
                Phase::Ready(i) => self.begin(i)?,
                Phase::Memory(t) => {
                    let width = if t.size == Size::Byte {
                        Width::Byte
                    } else {
                        Width::Word
                    };
                    let address = t.address.wrapping_add(u16::from(t.done));
                    return Ok(if t.store {
                        let shift = (u32::from(t.size.bytes())
                            - u32::from(t.done)
                            - u32::from(width.bytes()))
                            * 8;
                        Action::Write {
                            address,
                            width,
                            value: (t.value >> shift) as u16,
                            mov_byte: width == Width::Byte && !t.ccr,
                        }
                    } else {
                        Action::Read {
                            address,
                            width,
                            fetch: false,
                        }
                    });
                }
                Phase::BitRead { address, .. } => {
                    return Ok(Action::Read {
                        address,
                        width: Width::Byte,
                        fetch: false,
                    })
                }
                Phase::BitWrite { address, value } => {
                    return Ok(Action::Write {
                        address,
                        width: Width::Byte,
                        value: u16::from(value),
                        mov_byte: false,
                    })
                }
                Phase::IndirectJump { address, .. } => {
                    return Ok(Action::Read {
                        address: address & !1,
                        width: Width::Word,
                        fetch: false,
                    })
                }
                Phase::Call {
                    address, return_pc, ..
                } => {
                    return Ok(Action::Write {
                        address: address & !1,
                        width: Width::Word,
                        value: return_pc,
                        mov_byte: false,
                    })
                }
                Phase::BranchWait { .. } => return Ok(Action::Idle(2)),
                Phase::ReturnPc | Phase::ReturnCcr | Phase::ReturnExceptionPc { .. } => {
                    return Ok(Action::Read {
                        address: self.registers.sp() & !1,
                        width: Width::Word,
                        fetch: false,
                    })
                }
                Phase::ExceptionPc { pc, .. } => {
                    return Ok(Action::Write {
                        address: self.registers.sp() & !1,
                        width: Width::Word,
                        value: pc,
                        mov_byte: false,
                    })
                }
                Phase::ExceptionCcr { ccr, .. } => {
                    return Ok(Action::Write {
                        address: self.registers.sp() & !1,
                        width: Width::Word,
                        value: u16::from(ccr) << 8,
                        mov_byte: false,
                    })
                }
                Phase::ExceptionVector { vector } => {
                    return Ok(Action::Read {
                        address: u16::from(vector) * 2,
                        width: Width::Word,
                        fetch: false,
                    })
                }
                Phase::MulDiv { states, .. } => return Ok(Action::Idle(states)),
                Phase::Copy { stage, value, .. } => {
                    let source = self.registers.er[5] as u16;
                    let dest = self.registers.er[6] as u16;
                    return Ok(match stage {
                        0 | 2 => Action::Read {
                            address: source,
                            width: Width::Byte,
                            fetch: false,
                        },
                        1 => Action::Read {
                            address: dest,
                            width: Width::Byte,
                            fetch: false,
                        },
                        _ => Action::Write {
                            address: dest,
                            width: Width::Byte,
                            value: u16::from(value),
                            mov_byte: false,
                        },
                    });
                }
            }
        }
    }
    pub fn complete(&mut self, value: u16) -> Result<(), Error> {
        match self.phase {
            Phase::Fetch => {
                if self.word_count >= 5 {
                    return Err(Error::Internal("decoder requested more than ten bytes"));
                }
                self.words[usize::from(self.word_count)] = value;
                self.word_count += 1;
                self.registers.pc = self.registers.pc.wrapping_add(2);
                match decode::decode(&self.words[..usize::from(self.word_count)]) {
                    Decode::NeedWord => {}
                    Decode::Ready { instruction, words } => {
                        if words != self.word_count {
                            return Err(Error::Internal(
                                "decoder consumed a different fetch count",
                            ));
                        }
                        self.phase = Phase::Ready(instruction);
                    }
                    Decode::Invalid => {
                        return Err(Error::Decode {
                            pc: self.instruction_pc,
                            words: self.words,
                            count: self.word_count,
                        })
                    }
                }
            }
            Phase::Memory(mut t) => {
                let bytes = if t.size == Size::Byte { 1 } else { 2 };
                if !t.store {
                    t.value = if t.done == 0 {
                        u32::from(value)
                    } else {
                        (t.value << 16) | u32::from(value)
                    };
                }
                t.done += bytes;
                if u16::from(t.done) < t.size.bytes() {
                    self.phase = Phase::Memory(t);
                } else {
                    if let Some((r, v)) = t.post {
                        self.registers.er[usize::from(r)] = v;
                    }
                    if t.ccr {
                        if !t.store {
                            self.registers.ccr = (t.value >> 8) as u8;
                            self.interrupt_delay = 1;
                        }
                    } else {
                        if !t.store {
                            self.registers.write(t.size, t.register, t.value);
                        }
                        self.registers.ccr = alu::logical(t.value, t.size, self.registers.ccr);
                    }
                    self.finish();
                }
            }
            Phase::BitRead { address, op, bit } => {
                let (result, writes) = self.apply_bit(op, bit, value as u8);
                if writes {
                    self.phase = Phase::BitWrite {
                        address,
                        value: result,
                    };
                } else {
                    self.finish();
                }
            }
            Phase::BitWrite { .. } => self.finish(),
            Phase::IndirectJump { call, .. } => self.jump(value, call),
            Phase::Call { target, .. } => self.phase = Phase::BranchWait { target },
            Phase::BranchWait { target } => {
                self.registers.pc = target & !1;
                self.finish();
            }
            Phase::ReturnPc => {
                self.registers.move_sp(2);
                self.phase = Phase::BranchWait { target: value };
            }
            Phase::ReturnCcr => {
                self.registers.move_sp(2);
                self.phase = Phase::ReturnExceptionPc {
                    ccr: (value >> 8) as u8,
                };
            }
            Phase::ReturnExceptionPc { ccr } => {
                self.registers.move_sp(2);
                self.registers.ccr = ccr;
                self.interrupt_delay = 1;
                self.phase = Phase::BranchWait { target: value };
            }
            Phase::ExceptionPc { vector, ccr, .. } => {
                self.registers.move_sp(-2);
                self.phase = Phase::ExceptionCcr { vector, ccr };
            }
            Phase::ExceptionCcr { vector, .. } => self.phase = Phase::ExceptionVector { vector },
            Phase::ExceptionVector { .. } => {
                self.registers.pc = value & !1;
                self.phase = Phase::Boundary;
            }
            Phase::MulDiv {
                size,
                dst,
                value: result,
                flags,
                ..
            } => {
                self.registers.write(size, dst, result);
                self.registers.ccr = flags;
                self.finish();
            }
            Phase::Copy {
                word_count, stage, ..
            } => {
                let count_size = if word_count { Size::Word } else { Size::Byte };
                let field = if word_count { 4 } else { 12 };
                let count = self.registers.read(count_size, field);
                match stage {
                    0 => {
                        self.phase = Phase::Copy {
                            word_count,
                            stage: 1,
                            value: 0,
                        }
                    }
                    1 => {
                        if count == 0 {
                            self.finish();
                        } else {
                            self.phase = Phase::Copy {
                                word_count,
                                stage: 2,
                                value: 0,
                            };
                        }
                    }
                    2 => {
                        self.phase = Phase::Copy {
                            word_count,
                            stage: 3,
                            value: value as u8,
                        }
                    }
                    _ => {
                        self.registers.er[5] = self.registers.er[5].wrapping_add(1);
                        self.registers.er[6] = self.registers.er[6].wrapping_add(1);
                        self.registers
                            .write(count_size, field, count.wrapping_sub(1));
                        if count == 1 {
                            self.finish();
                        } else {
                            self.phase = Phase::Copy {
                                word_count,
                                stage: 2,
                                value: 0,
                            };
                        }
                    }
                }
            }
            _ => {
                return Err(Error::Internal(
                    "CPU completion without an outstanding timed action",
                ))
            }
        }
        Ok(())
    }
    fn jump(&mut self, target: u16, call: bool) {
        if call {
            let return_pc = self.registers.pc;
            self.registers.move_sp(-2);
            self.phase = Phase::Call {
                address: self.registers.sp(),
                target,
                return_pc,
            };
        } else {
            self.phase = Phase::BranchWait { target };
        }
    }
    fn apply_bit(&mut self, op: Bit, bit: u8, value: u8) -> (u8, bool) {
        let mask = 1u8 << (bit & 7);
        let selected = value & mask != 0;
        let old = self.registers.ccr & C != 0;
        match op {
            Bit::Set => (value | mask, true),
            Bit::Clear => (value & !mask, true),
            Bit::Not => (value ^ mask, true),
            Bit::Test => {
                self.registers.ccr = (self.registers.ccr & !Z) | if selected { 0 } else { Z };
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
                self.registers.ccr = (self.registers.ccr & !C) | if result { C } else { 0 };
                (value, false)
            }
        }
    }
    fn begin(&mut self, instruction: Instruction) -> Result<(), Error> {
        match instruction {
            Instruction::Nop => self.finish(),
            Instruction::Sleep => {
                self.retired = self.retired.wrapping_add(1);
                self.phase = Phase::Sleeping;
            }
            Instruction::Binary { op, size, dst, src } => {
                let a = self.registers.read(size, dst);
                let b = self.source(size, src);
                let (r, f) = alu::binary(op, size, a, b, self.registers.ccr);
                if op != Alu::Cmp {
                    self.registers.write(size, dst, r);
                }
                self.registers.ccr = f;
                self.finish();
            }
            Instruction::Unary {
                op,
                size,
                dst,
                amount,
            } => {
                let (r, f) = alu::unary(
                    op,
                    size,
                    self.registers.read(size, dst),
                    amount,
                    self.registers.ccr,
                );
                self.registers.write(size, dst, r);
                self.registers.ccr = f;
                self.finish();
            }
            Instruction::Shift { op, size, dst } => {
                let (r, f) =
                    alu::shift(op, size, self.registers.read(size, dst), self.registers.ccr);
                self.registers.write(size, dst, r);
                self.registers.ccr = f;
                self.finish();
            }
            Instruction::Quick { dst, delta } => {
                self.registers.er[usize::from(dst)] =
                    self.registers.er[usize::from(dst)].wrapping_add(i32::from(delta) as u32);
                self.finish();
            }
            Instruction::Bit { op, bit, target } => {
                let bit = self.source(Size::Byte, bit) as u8 & 7;
                match target {
                    Target::Reg(r) => {
                        let (result, writes) =
                            self.apply_bit(op, bit, self.registers.read(Size::Byte, r) as u8);
                        if writes {
                            self.registers.write(Size::Byte, r, u32::from(result));
                        }
                        self.finish();
                    }
                    Target::Memory(address) => {
                        let (address, _) = self.target_address(address, Size::Byte);
                        self.phase = Phase::BitRead { address, op, bit };
                    }
                }
            }
            Instruction::Memory {
                size,
                reg,
                address,
                store,
                ccr,
            } => {
                let value = if ccr {
                    u32::from(self.registers.ccr) << 8
                } else {
                    self.registers.read(size, reg)
                };
                let (mut address, post) = self.target_address(address, size);
                if size != Size::Byte {
                    address &= !1;
                }
                self.phase = Phase::Memory(Transfer {
                    address,
                    size,
                    register: reg,
                    store,
                    ccr,
                    value: if store { value } else { 0 },
                    done: 0,
                    post,
                });
            }
            Instruction::Branch {
                condition,
                displacement,
            } => {
                let target = if alu::condition(condition, self.registers.ccr) {
                    self.registers.pc.wrapping_add(displacement as u16)
                } else {
                    self.registers.pc
                };
                self.phase = Phase::BranchWait { target };
            }
            Instruction::BranchSubroutine(displacement) => {
                self.jump(self.registers.pc.wrapping_add(displacement as u16), true)
            }
            Instruction::Jump { target, call } => match target {
                Jump::Absolute(addr) => self.jump(addr, call),
                Jump::Register(r) => self.jump(self.registers.er[usize::from(r)] as u16, call),
                Jump::Vector(addr) => {
                    self.phase = Phase::IndirectJump {
                        address: u16::from(addr),
                        call,
                    }
                }
            },
            Instruction::Return { exception } => {
                self.phase = if exception {
                    Phase::ReturnCcr
                } else {
                    Phase::ReturnPc
                }
            }
            Instruction::Trap(vector) => {
                self.retired = self.retired.wrapping_add(1);
                self.enter_exception(8 + vector);
            }
            Instruction::Ccr { op, source } => {
                let v = self.source(Size::Byte, source) as u8;
                self.registers.ccr = match op {
                    CcrOp::Load => v,
                    CcrOp::And => self.registers.ccr & v,
                    CcrOp::Or => self.registers.ccr | v,
                    CcrOp::Xor => self.registers.ccr ^ v,
                };
                self.interrupt_delay = 1;
                self.finish();
            }
            Instruction::StoreCcr(r) => {
                self.registers
                    .write(Size::Byte, r, u32::from(self.registers.ccr));
                self.finish();
            }
            Instruction::MulDiv {
                divide,
                signed,
                size,
                src,
                dst,
            } => self.muldiv(divide, signed, size, src, dst)?,
            Instruction::Decimal { subtract, reg } => {
                let a = self.registers.read(Size::Byte, reg) as u8;
                let h = self.registers.ccr & H != 0;
                let c = self.registers.ccr & C != 0;
                let correction = if subtract {
                    (if h { 6u8 } else { 0 }) + if c { 0x60 } else { 0 }
                } else {
                    (if h || a & 15 > 9 { 6u8 } else { 0 }) + if c || a > 0x99 { 0x60 } else { 0 }
                };
                let r = if subtract {
                    a.wrapping_sub(correction)
                } else {
                    a.wrapping_add(correction)
                };
                self.registers.write(Size::Byte, reg, u32::from(r));
                // H and V are not guaranteed by this instruction. Preserve a
                // deterministic witness rather than inventing arithmetic facts.
                self.registers.ccr =
                    (self.registers.ccr & !(N | Z)) | alu::nz(u32::from(r), Size::Byte);
                if !subtract && (u16::from(a) + u16::from(correction) > 255) {
                    self.registers.ccr |= C;
                }
                self.finish();
            }
            Instruction::EepMov { word_count } => {
                self.phase = Phase::Copy {
                    word_count,
                    stage: 0,
                    value: 0,
                }
            }
        }
        Ok(())
    }
    fn muldiv(
        &mut self,
        divide: bool,
        signed: bool,
        size: Size,
        src: u8,
        dst: u8,
    ) -> Result<(), Error> {
        let out_size = if size == Size::Byte {
            Size::Word
        } else {
            Size::Long
        };
        let divisor = self.registers.read(size, src);
        let dividend = self.registers.read(out_size, dst);
        let mut flags = self.registers.ccr;
        let value;
        if !divide {
            let low = dividend & size.mask();
            value = if signed {
                let a = if size == Size::Byte {
                    divisor as u8 as i8 as i64
                } else {
                    divisor as u16 as i16 as i64
                };
                let b = if size == Size::Byte {
                    low as u8 as i8 as i64
                } else {
                    low as u16 as i16 as i64
                };
                (a * b) as u32 & out_size.mask()
            } else {
                divisor.wrapping_mul(low) & out_size.mask()
            };
            if signed {
                flags = (flags & !(N | Z)) | alu::nz(value, out_size);
            }
        } else {
            if divisor == 0 {
                return Err(Error::Unsupported {
                    component: "CPU division",
                    detail: "division by zero has no characterized silicon witness",
                    address: self.instruction_pc,
                });
            }
            let (q, r, negative) = if signed {
                let a = if size == Size::Byte {
                    dividend as u16 as i16 as i64
                } else {
                    dividend as i32 as i64
                };
                let b = if size == Size::Byte {
                    divisor as u8 as i8 as i64
                } else {
                    divisor as u16 as i16 as i64
                };
                let q = a / b;
                let r = a % b;
                let min = -(1i64 << (size.bits() - 1));
                let max = (1i64 << (size.bits() - 1)) - 1;
                if q < min || q > max {
                    return Err(Error::Unsupported {
                        component: "CPU division",
                        detail: "signed quotient overflow has no characterized silicon witness",
                        address: self.instruction_pc,
                    });
                }
                (q as u32, r as u32, (a < 0) != (b < 0))
            } else {
                let q = dividend / divisor;
                let r = dividend % divisor;
                if q > size.mask() {
                    return Err(Error::Unsupported {
                        component: "CPU division",
                        detail: "unsigned quotient overflow has no characterized silicon witness",
                        address: self.instruction_pc,
                    });
                }
                (q, r, divisor & size.sign() != 0)
            };
            value = ((r & size.mask()) << size.bits()) | (q & size.mask());
            flags = (flags & !(N | Z)) | if negative { N } else { 0 };
        }
        // Fetches are charged by the bus. Remaining nominal internal states:
        // unsigned byte 14 total, unsigned word 22; signed adds a prefix fetch.
        let total: u32 = if divide {
            if size == Size::Byte {
                14
            } else {
                22
            }
        } else if size == Size::Byte {
            14
        } else {
            22
        };
        let total = total + if signed { 2 } else { 0 };
        self.phase = Phase::MulDiv {
            size: out_size,
            dst,
            value,
            flags,
            states: total.saturating_sub(u32::from(self.word_count) * 2),
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_preserve_unwritten_bits() {
        let mut r = Registers {
            er: [0x11223344; 8],
            pc: 0,
            ccr: 0,
        };
        r.write(Size::Byte, 0, 0xaa);
        assert_eq!(r.er[0], 0x1122aa44);
        r.write(Size::Byte, 8, 0xbb);
        assert_eq!(r.er[0], 0x1122aabb);
        r.write(Size::Word, 8, 0xccdd);
        assert_eq!(r.er[0], 0xccddaabb);
    }
    #[test]
    fn executes_reset_prologue_without_special_addresses() {
        let code = [0x7907, 0xff80, 0x1b97, 0xf801, 0x8802, 0x0180];
        let mut cpu = Cpu::new(0);
        for _ in 0..20 {
            match cpu.next(None).unwrap() {
                Action::Read {
                    address,
                    fetch: true,
                    ..
                } => cpu.complete(code[usize::from(address / 2)]).unwrap(),
                Action::Idle(_) => cpu.complete(0).unwrap(),
                Action::Sleep => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(cpu.registers.sp(), 0xff7c);
        assert_eq!(cpu.registers.read(Size::Byte, 8), 3);
        assert!(cpu.sleeping());
    }
    #[test]
    fn exception_writes_pc_before_ccr() {
        let mut cpu = Cpu::new(0x1234);
        cpu.registers.er[7] = 0xff7c;
        cpu.registers.ccr = 0x21;
        assert_eq!(
            cpu.next(Some(25)).unwrap(),
            Action::Write {
                address: 0xff7a,
                width: Width::Word,
                value: 0x1234,
                mov_byte: false
            }
        );
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(None).unwrap(),
            Action::Write {
                address: 0xff78,
                width: Width::Word,
                value: 0x2100,
                mov_byte: false
            }
        );
    }
}
