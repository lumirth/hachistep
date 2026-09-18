//! A single resumable execution engine. `Action` describes the next physical
//! access or internal wait; `complete` commits that action.
pub mod alu;
pub mod decode;
pub(crate) mod state;
use crate::error::Error;
use alu::{C, H, I, N, Z};
use decode::{Address, Alu, Bit, CcrOp, Decode, Instruction, Jump, Size, Source, Target};

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum Width {
    Byte = 0,
    Word = 1,
}
impl Width {
    pub const fn bytes(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
        }
    }
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum Action {
    Read {
        address: u16,
        width: Width,
        fetch: bool,
    } = 0,
    Write {
        address: u16,
        width: Width,
        value: u16,
        mov_byte: bool,
    } = 1,
    Idle(u32) = 2,
    Sleep = 3,
}
/// Only the instruction provenance used by register hardware. Word-store
/// lanes and bit-operation writebacks are not MOV.B accesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOrigin {
    Other,
    MovByte,
    MovByteAbs8 { pc_bit1: bool },
}
impl WriteOrigin {
    pub fn is_mov(self) -> bool {
        self != Self::Other
    }
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Debug, PartialEq, Eq)]
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

#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct Transfer {
    address: u16,
    size: Size,
    register: u8,
    store: bool,
    ccr: bool,
    absolute8: bool,
    value: u32,
    done: u8,
    post: Option<(u8, u32)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    ResetVector,
    Boundary,
    Fetch,
    Ready(Instruction),
    Execute(Instruction),
    Prefetch {
        address: u16,
        retain: bool,
    },
    Delay(u32),
    Finish,
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
        fetch_target: bool,
    },
    BranchTarget {
        target: u16,
        take: bool,
    },
    JumpTarget {
        target: u16,
        call: bool,
        return_pc: u16,
    },
    BranchWait {
        target: u16,
        call: bool,
    },
    EntryWait {
        target: u16,
    },
    EntryTarget {
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
    continuation: Phase,
    prefetch: Option<(u16, u16)>,
    accepted_vector: Option<u8>,
    pub(crate) words: [u16; 5],
    pub(crate) word_count: u8,
    pub(crate) instruction_pc: u16,
    pub retired: u64,
    pub interrupt_entries: u64,
    interrupt_delay: u8,
}
impl Cpu {
    /// The caller obtains the reset vector through the MCU bus.
    /// The model initializes general registers to zero at cold startup.
    pub fn new(reset_vector: u16) -> Self {
        Self {
            registers: Registers {
                er: [0; 8],
                pc: reset_vector & !1,
                ccr: I,
            },
            phase: Phase::Boundary,
            continuation: Phase::Boundary,
            prefetch: None,
            accepted_vector: None,
            words: [0; 5],
            word_count: 0,
            instruction_pc: reset_vector & !1,
            retired: 0,
            interrupt_entries: 0,
            interrupt_delay: 0,
        }
    }
    /// Power/reset entry performs the vector read and pipeline fill on the bus.
    pub(crate) fn reset() -> Self {
        let mut cpu = Self::new(0);
        cpu.phase = Phase::ResetVector;
        cpu.interrupt_delay = 1; // The first reset instruction runs even with NMI pending.
        cpu
    }
    pub fn sleeping(&self) -> bool {
        self.phase == Phase::Sleeping
    }
    /// Integration acknowledges a latched request only after the CPU selected
    /// its exception, never merely because a request was offered to `next`.
    pub(crate) fn take_accepted_vector(&mut self) -> Option<u8> {
        self.accepted_vector.take()
    }
    pub fn boundary(&self) -> bool {
        matches!(self.phase, Phase::Boundary | Phase::Sleeping)
    }
    pub fn instruction_pc(&self) -> u16 {
        self.instruction_pc
    }
    pub(crate) fn write_origin(&self, mov_byte: bool) -> WriteOrigin {
        if !mov_byte {
            WriteOrigin::Other
        } else if matches!(
            self.phase,
            Phase::Memory(Transfer {
                absolute8: true,
                ..
            })
        ) {
            WriteOrigin::MovByteAbs8 {
                pc_bit1: self.instruction_pc & 2 != 0,
            }
        } else {
            WriteOrigin::MovByte
        }
    }
    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::ResetVector => "reset-vector",
            Phase::Boundary => "boundary",
            Phase::Fetch => "fetch",
            Phase::Ready(_) => "decoded",
            Phase::Execute(_) => "execute",
            Phase::Prefetch { .. } => "prefetch",
            Phase::Delay(_) => "internal",
            Phase::Finish => "finish",
            Phase::Memory(_) => "memory",
            Phase::BitRead { .. } => "bit-read",
            Phase::BitWrite { .. } => "bit-write",
            Phase::IndirectJump { .. } => "indirect-jump",
            Phase::Call { .. } => "call-stack",
            Phase::BranchWait { .. } => "branch-wait",
            Phase::BranchTarget { .. } => "branch-target",
            Phase::JumpTarget { .. } => "jump-target",
            Phase::EntryWait { .. } => "entry-wait",
            Phase::EntryTarget { .. } => "entry-target",
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
    fn prefetch_then(&mut self, address: u16, retain: bool, next: Phase) {
        self.continuation = next;
        self.phase = Phase::Prefetch {
            address: address & !1,
            retain,
        };
    }
    fn delay_then(&mut self, states: u32, next: Phase) {
        self.continuation = next;
        self.phase = Phase::Delay(states);
    }
    fn continue_execution(&mut self) -> Result<(), Error> {
        self.phase = core::mem::replace(&mut self.continuation, Phase::Boundary);
        match self.phase {
            Phase::Execute(i) => self.begin(i)?,
            Phase::Finish => self.finish(),
            _ => {}
        }
        Ok(())
    }
    fn prepare(&mut self, i: Instruction) -> Result<(), Error> {
        // REJ09B0213-0300 §2.8: NEXT is a real retained bus fetch. Memory
        // bit operations and EEPMOV fetch it later in their own sequences.
        match i {
            Instruction::Bit {
                target: Target::Memory(_),
                ..
            }
            | Instruction::EepMov { .. }
            | Instruction::Jump {
                target: Jump::Absolute(_),
                ..
            } => self.begin(i)?,
            Instruction::Branch { .. } | Instruction::BranchSubroutine(_)
                if self.word_count == 2 =>
            {
                self.delay_then(2, Phase::Execute(i))
            }
            _ => {
                let retain = !matches!(
                    i,
                    Instruction::Jump { .. }
                        | Instruction::BranchSubroutine(_)
                        | Instruction::Return { .. }
                        | Instruction::Trap(_)
                );
                self.prefetch_then(self.registers.pc, retain, Phase::Execute(i));
            }
        }
        Ok(())
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
        self.enter_exception(13, false);
        Ok(())
    }
    fn enter_exception(&mut self, vector: u8, trap: bool) {
        let sleeping = self.sleeping();
        let pc = self.registers.pc;
        let ccr = self.registers.ccr;
        self.registers.ccr |= I;
        self.interrupt_entries = self.interrupt_entries.wrapping_add(1);
        self.accepted_vector = Some(vector);
        self.prefetch = None;
        self.registers.move_sp(-2);
        let stack = Phase::ExceptionPc { vector, pc, ccr };
        if trap || sleeping {
            // A sleeping CPU substitutes two internal states for the discarded
            // fetch. TRAPA has already fetched NEXT.
            self.delay_then(if sleeping { 4 } else { 2 }, stack);
        } else {
            self.prefetch_then(pc.wrapping_add(2), false, stack);
        }
    }
    /// Inspect the admitted action without changing CPU state.
    /// The request remains stable until `complete` is called.
    #[inline]
    pub(crate) fn issued_action(&self) -> Option<Action> {
        match self.phase {
            Phase::ResetVector => Some(Action::Read {
                address: 0,
                width: Width::Word,
                fetch: false,
            }),
            Phase::Fetch => Some(Action::Read {
                address: self.registers.pc & !1,
                width: Width::Word,
                fetch: true,
            }),
            Phase::Prefetch { address, .. }
            | Phase::BranchTarget {
                target: address, ..
            }
            | Phase::JumpTarget {
                target: address, ..
            }
            | Phase::EntryTarget { target: address } => Some(Action::Read {
                address: address & !1,
                width: Width::Word,
                fetch: true,
            }),
            Phase::Delay(states) => Some(Action::Idle(states)),
            Phase::Memory(t) => {
                let width = if t.size == Size::Byte {
                    Width::Byte
                } else {
                    Width::Word
                };
                let address = t.address.wrapping_add(u16::from(t.done));
                Some(if t.store {
                    let shift =
                        (u32::from(t.size.bytes()) - u32::from(t.done) - u32::from(width.bytes()))
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
                })
            }
            Phase::BitRead { address, .. } => Some(Action::Read {
                address,
                width: Width::Byte,
                fetch: false,
            }),
            Phase::BitWrite { address, value } => Some(Action::Write {
                address,
                width: Width::Byte,
                value: u16::from(value),
                mov_byte: false,
            }),
            Phase::IndirectJump { address, .. } => Some(Action::Read {
                address: address & !1,
                width: Width::Word,
                fetch: false,
            }),
            Phase::Call {
                address, return_pc, ..
            } => Some(Action::Write {
                address: address & !1,
                width: Width::Word,
                value: return_pc,
                mov_byte: false,
            }),
            Phase::BranchWait { .. } | Phase::EntryWait { .. } => Some(Action::Idle(2)),
            Phase::ReturnPc | Phase::ReturnCcr | Phase::ReturnExceptionPc { .. } => {
                Some(Action::Read {
                    address: self.registers.sp() & !1,
                    width: Width::Word,
                    fetch: false,
                })
            }
            Phase::ExceptionPc { pc, .. } => Some(Action::Write {
                address: self.registers.sp() & !1,
                width: Width::Word,
                value: pc,
                mov_byte: false,
            }),
            Phase::ExceptionCcr { ccr, .. } => Some(Action::Write {
                address: self.registers.sp() & !1,
                width: Width::Word,
                value: u16::from(ccr) * 0x0101,
                mov_byte: false,
            }),
            Phase::ExceptionVector { vector } => Some(Action::Read {
                address: u16::from(vector) * 2,
                width: Width::Word,
                fetch: false,
            }),
            Phase::MulDiv { states, .. } => Some(Action::Idle(states)),
            Phase::Copy {
                stage: stage @ (0 | 1 | 3 | 4),
                value,
                ..
            } => {
                let source = self.registers.er[5] as u16;
                let dest = self.registers.er[6] as u16;
                Some(match stage {
                    0 | 4 => Action::Read {
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
                })
            }
            _ => None,
        }
    }
    pub fn next(&mut self, interrupt: Option<u8>) -> Result<Action, Error> {
        loop {
            if let Some(action) = self.issued_action() {
                return Ok(action);
            }
            match self.phase {
                Phase::Boundary | Phase::Sleeping => {
                    if self.interrupt_delay == 0 {
                        if let Some(v) = interrupt {
                            if v == 7 || self.registers.ccr & I == 0 {
                                self.enter_exception(v, false);
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
                    if let Some((address, word)) = self.prefetch.take() {
                        if address == self.registers.pc {
                            self.complete(word)?;
                        }
                    }
                }
                Phase::Ready(i) => self.prepare(i)?,
                Phase::Execute(i) => self.begin(i)?,
                Phase::Finish => self.finish(),
                Phase::Copy {
                    word_count,
                    stage: 2,
                    value,
                } => {
                    if word_count && interrupt == Some(7) {
                        // REJ09B0152-0300 §3.8.6: .W accepts NMI at a break
                        // between transfer cycles, saves the NEXT instruction,
                        // and leaves R4/ER5/ER6 describing the remaining copy.
                        // .B defers even NMI; an issued read/write pair is never
                        // split here. Resumption requires the firmware loop.
                        self.finish();
                        self.enter_exception(7, false);
                        continue;
                    }
                    {
                        // Commit the admission decision before exposing a read.
                        // Repeating next() must not replace that outstanding
                        // physical request when a new interrupt is offered.
                        self.phase = Phase::Copy {
                            word_count,
                            stage: 4,
                            value,
                        };
                        continue;
                    }
                }
                _ => return Err(Error::Internal("CPU phase without issued work")),
            }
        }
    }
    pub fn complete(&mut self, value: u16) -> Result<(), Error> {
        match self.phase {
            Phase::ResetVector => self.phase = Phase::EntryWait { target: value },
            Phase::Prefetch { address, retain } => {
                if retain {
                    self.prefetch = Some((address, value));
                }
                if matches!(self.continuation, Phase::ExceptionPc { .. }) {
                    self.phase = Phase::Delay(2);
                } else {
                    self.continue_execution()?;
                }
            }
            Phase::Delay(_) => self.continue_execution()?,
            Phase::BranchTarget { target, take } => {
                if take {
                    self.registers.pc = target & !1;
                    self.prefetch = Some((target & !1, value));
                }
                self.finish();
            }
            Phase::JumpTarget {
                target,
                call,
                return_pc,
            } => {
                self.registers.pc = target & !1;
                self.prefetch = Some((target & !1, value));
                if call {
                    self.registers.move_sp(-2);
                    self.phase = Phase::Call {
                        address: self.registers.sp(),
                        target,
                        return_pc,
                        fetch_target: false,
                    };
                } else {
                    self.finish();
                }
            }
            Phase::EntryWait { target } => self.phase = Phase::EntryTarget { target },
            Phase::EntryTarget { target } => {
                self.registers.pc = target & !1;
                self.prefetch = Some((target & !1, value));
                self.phase = Phase::Boundary;
            }
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
                let next = if writes {
                    Phase::BitWrite {
                        address,
                        value: result,
                    }
                } else {
                    Phase::Finish
                };
                self.prefetch_then(self.registers.pc, true, next);
            }
            Phase::BitWrite { .. } => self.finish(),
            Phase::IndirectJump { call, .. } => {
                if call {
                    self.registers.move_sp(-2);
                    self.phase = Phase::Call {
                        address: self.registers.sp(),
                        target: value,
                        return_pc: self.registers.pc,
                        fetch_target: true,
                    };
                } else {
                    self.jump(value, false);
                }
            }
            Phase::Call {
                target,
                fetch_target,
                ..
            } => {
                if fetch_target {
                    self.phase = Phase::JumpTarget {
                        target,
                        call: false,
                        return_pc: 0,
                    };
                } else {
                    self.finish();
                }
            }
            Phase::BranchWait { target, call } => {
                self.phase = Phase::JumpTarget {
                    target,
                    call,
                    return_pc: self.registers.pc,
                }
            }
            Phase::ReturnPc => {
                self.registers.move_sp(2);
                self.jump(value, false);
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
                // RTE is not one of §3.8.5's CCR-writing deferral instructions.
                self.jump(value, false);
            }
            Phase::ExceptionPc { vector, ccr, .. } => {
                self.registers.move_sp(-2);
                self.phase = Phase::ExceptionCcr { vector, ccr };
            }
            Phase::ExceptionCcr { vector, .. } => self.phase = Phase::ExceptionVector { vector },
            Phase::ExceptionVector { .. } => {
                self.phase = Phase::EntryWait { target: value };
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
                            self.prefetch_then(self.registers.pc, true, Phase::Finish);
                        } else {
                            self.phase = Phase::Copy {
                                word_count,
                                stage: 2,
                                value: 0,
                            };
                        }
                    }
                    4 => {
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
                            self.prefetch_then(self.registers.pc, true, Phase::Finish);
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
        self.phase = Phase::BranchWait { target, call };
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
                // MOV @-ERn updates the full address register before reading
                // an aliased source (RnH/RnL/Rn/En/ERn). ADE-602-053A
                // MOV.B/W/L usage notes, pp. 121/123/125. Loads still commit
                // a post-increment before replacing the destination field.
                let updating = matches!(
                    address,
                    Address::PreDecrement(_) | Address::PostIncrement(_)
                );
                let absolute8 = matches!(address, Address::Absolute(_)) && self.word_count == 1;
                let (mut address, post) = self.target_address(address, size);
                let value = if ccr {
                    u32::from(self.registers.ccr) << 8
                } else {
                    self.registers.read(size, reg)
                };
                if size != Size::Byte {
                    address &= !1;
                }
                let transfer = Phase::Memory(Transfer {
                    address,
                    size,
                    register: reg,
                    store,
                    ccr,
                    absolute8,
                    value: if store { value } else { 0 },
                    done: 0,
                    post,
                });
                if updating {
                    self.delay_then(2, transfer);
                } else {
                    self.phase = transfer;
                }
            }
            Instruction::Branch {
                condition,
                displacement,
            } => {
                let taken = alu::condition(condition, self.registers.ccr);
                // Bcc8 reads both fallthrough and target, even when untaken.
                // Bcc16 instead has an address-calculation interval followed
                // by the selected next fetch (§2.8, pp.235–236).
                let target = if self.word_count == 2 && !taken {
                    self.registers.pc
                } else {
                    self.registers.pc.wrapping_add(displacement as u16)
                };
                self.phase = Phase::BranchTarget {
                    target,
                    take: taken || self.word_count == 2,
                };
            }
            Instruction::BranchSubroutine(displacement) => {
                self.phase = Phase::JumpTarget {
                    target: self.registers.pc.wrapping_add(displacement as u16),
                    call: true,
                    return_pc: self.registers.pc,
                };
            }
            Instruction::Jump { target, call } => match target {
                Jump::Absolute(addr) => self.jump(addr, call),
                Jump::Register(r) => {
                    self.phase = Phase::JumpTarget {
                        target: self.registers.er[usize::from(r)] as u16,
                        call,
                        return_pc: self.registers.pc,
                    }
                }
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
                self.enter_exception(8 + vector, true);
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
                // The model preserves H and V, whose results are unspecified.
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
            // §2.2.26–27: neither zero division nor overflow raises an
            // exception. N follows operand signs (even for a zero quotient),
            // and Z reports a zero divisor. Undefined destination bits use a
            // deterministic continuation: retain on zero, narrow on overflow.
            let negative = if signed {
                (dividend & out_size.sign() != 0) != (divisor & size.sign() != 0)
            } else {
                divisor & size.sign() != 0
            };
            flags = (flags & !(N | Z))
                | if negative { N } else { 0 }
                | if divisor == 0 { Z } else { 0 };
            value = if divisor == 0 {
                dividend
            } else {
                let (q, r) = if signed {
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
                    ((a / b) as u32, (a % b) as u32)
                } else {
                    (dividend / divisor, dividend % divisor)
                };
                ((r & size.mask()) << size.bits()) | (q & size.mask())
            };
        }
        // The prefix and NEXT fetches are physical bus actions. The table
        // assigns a further 12/20 internal states for byte/word operands.
        self.phase = Phase::MulDiv {
            size: out_size,
            dst,
            value,
            flags,
            states: if size == Size::Byte { 12 } else { 20 },
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
                } => cpu
                    .complete(code.get(usize::from(address / 2)).copied().unwrap_or(0))
                    .unwrap(),
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
            Action::Read {
                address: 0x1236,
                width: Width::Word,
                fetch: true,
            }
        );
        cpu.complete(0).unwrap();
        assert_eq!(cpu.next(None).unwrap(), Action::Idle(2));
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(None).unwrap(),
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
                value: 0x2121,
                mov_byte: false
            }
        );
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(None).unwrap(),
            Action::Read {
                address: 50,
                width: Width::Word,
                fetch: false,
            }
        );
        cpu.complete(0x200).unwrap();
        assert_eq!(cpu.next(None).unwrap(), Action::Idle(2));
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(None).unwrap(),
            Action::Read {
                address: 0x200,
                width: Width::Word,
                fetch: true,
            }
        );
        cpu.complete(0).unwrap();
        assert!(cpu.boundary());
        assert_eq!(cpu.registers.pc, 0x200);
        assert_eq!(cpu.retired, 0);
        assert_eq!(cpu.interrupt_entries, 1);
    }
}
