//! A single resumable execution engine. `Action` describes the next physical
//! access or internal wait; `complete` commits that action.
pub mod alu;
pub mod decode;
pub(crate) mod execution;
pub(crate) mod state;
use crate::error::Error;
use alu::{C, H, I, N, Z};
use decode::{Address, Alu, Bit, BitIndex, CcrOp, Decode, Instruction, Jump, Size, Source, Target};
use execution::{Bus, Exit, IntervalBus, Projection, Reply, Stop};

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
/// An issued request and the admission effects that precede it. The MCU
/// expires retained enables after the CPU has sampled the offered interrupt.
pub(crate) struct Request {
    pub action: Action,
    pub admission: bool,
    pub exception: Option<u8>,
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
    pub(crate) fn read(&self, size: Size, field: u8) -> u32 {
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
    pub(crate) fn write(&mut self, size: Size, field: u8, value: u32) {
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
impl Transfer {
    #[inline]
    fn transact(self, bus: &mut impl Bus) -> Result<u16, Stop> {
        let width = if self.size == Size::Byte {
            Width::Byte
        } else {
            Width::Word
        };
        let address = self.address.wrapping_add(u16::from(self.done));
        if self.store {
            let shift =
                (u32::from(self.size.bytes()) - u32::from(self.done) - u32::from(width.bytes()))
                    * 8;
            bus.write(
                address,
                width,
                (self.value >> shift) as u16,
                width == Width::Byte && !self.ccr,
            )?;
            Ok(0)
        } else {
            bus.read(address, width, false)
        }
    }
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
    #[cfg(feature = "profile-work")]
    pub(crate) phase_dispatches: crate::profile_work::Counter,
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
            #[cfg(feature = "profile-work")]
            phase_dispatches: Default::default(),
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
        matches!(self.phase, Phase::Sleeping)
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
    /// Accept physical or retained fetch bytes through the same decoder bookkeeping.
    /// The caller selects the instruction sequence; no synthetic bus completion occurs.
    fn accept_fetched(&mut self, value: u16) -> Result<Option<Instruction>, Error> {
        if self.word_count >= 5 {
            return Err(Error::Internal("decoder requested more than ten bytes"));
        }
        self.words[usize::from(self.word_count)] = value;
        self.word_count += 1;
        self.registers.pc = self.registers.pc.wrapping_add(2);
        match decode::decode(&self.words[..usize::from(self.word_count)]) {
            Decode::NeedWord => self.phase = Phase::Fetch,
            Decode::Ready { instruction, words } => {
                if words != self.word_count {
                    return Err(Error::Internal("decoder consumed a different fetch count"));
                }
                return Ok(Some(instruction));
            }
            Decode::Invalid => {
                return Err(Error::Decode {
                    pc: self.instruction_pc,
                    words: self.words,
                    count: self.word_count,
                })
            }
        }
        Ok(None)
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
    /// Project the same physical operation used by the interval executor.
    /// Projection stops before the arm can mutate any CPU state.
    pub(crate) fn issued_action(&mut self) -> Option<Action> {
        match self.execute_phase(&mut Projection) {
            Err(Stop::Request(action)) => Some(action),
            _ => None,
        }
    }
    /// Sample pending requests only at a hardware admission point.
    pub fn next(&mut self, interrupt: impl FnMut() -> Option<u8>) -> Result<Action, Error> {
        self.next_inner(interrupt)
    }
    #[inline(always)]
    fn next_inner(&mut self, interrupt: impl FnMut() -> Option<u8>) -> Result<Action, Error> {
        self.prepare_work(interrupt)?;
        self.issued_action()
            .or_else(|| self.sleeping().then_some(Action::Sleep))
            .ok_or(Error::Internal("CPU phase without issued work"))
    }
    /// Sample the offer before the MCU expires retained enables. Sleeping and
    /// accepted exceptions retain their own progress; normal admission consumes
    /// actual prefetched bytes without invoking the physical-phase dispatcher.
    fn admit(
        &mut self,
        mut interrupt: impl FnMut() -> Option<u8>,
    ) -> Result<Option<Instruction>, Error> {
        if self.interrupt_delay == 0 {
            if let Some(v) = interrupt() {
                if v == 7 || self.registers.ccr & I == 0 {
                    self.enter_exception(v, false);
                    return Ok(None);
                }
            }
        }
        if matches!(self.phase, Phase::Sleeping) {
            return Ok(None);
        }
        self.interrupt_delay = self.interrupt_delay.saturating_sub(1);
        self.instruction_pc = self.registers.pc;
        self.word_count = 0;
        self.words = [0; 5];
        self.phase = Phase::Fetch;
        if let Some((address, word)) = self.prefetch.take() {
            if address == self.registers.pc {
                return self.accept_fetched(word);
            }
        }
        Ok(None)
    }
    fn prepare_work(&mut self, mut interrupt: impl FnMut() -> Option<u8>) -> Result<(), Error> {
        loop {
            match self.phase {
                Phase::Boundary | Phase::Sleeping => {
                    if let Some(instruction) = self.admit(&mut interrupt)? {
                        self.prepare(instruction)?;
                    }
                    if self.sleeping() {
                        return Ok(());
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
                    if word_count && interrupt() == Some(7) {
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
                _ => return Ok(()),
            }
        }
    }
    pub fn complete(&mut self, value: u16) -> Result<(), Error> {
        match self.execute_phase(&mut Reply(value)) {
            Ok(true) => Ok(()),
            Err(Stop::Core(error)) => Err(error),
            _ => Err(Error::Internal(
                "CPU completion without an outstanding timed action",
            )),
        }
    }
    /// Execute physical effects locally; only a genuine fence publishes work
    /// to Machine. Successful ordinary accesses never construct an Action.
    #[inline]
    pub(crate) fn run_interval(&mut self, bus: &mut impl IntervalBus) -> Result<Exit, Error> {
        loop {
            let result = if self.boundary() {
                let instruction = self.admit(|| bus.interrupt())?;
                bus.instruction_boundary();
                if self.accepted_vector.is_some() {
                    return self.exception_exit();
                }
                if self.sleeping() {
                    return Ok(Exit::Sleep);
                }
                match instruction {
                    Some(instruction) => self.run_instruction(instruction, bus, true),
                    None => self.execute_phase(bus),
                }
            } else {
                self.execute_phase(bus)
            };
            match result {
                Ok(true) => {
                    if self.accepted_vector.is_some() {
                        return self.exception_exit();
                    }
                }
                Ok(false) => {
                    let admission = self.boundary();
                    self.prepare_work(|| bus.interrupt())?;
                    if admission {
                        bus.instruction_boundary();
                    }
                    if self.accepted_vector.is_some() {
                        return self.exception_exit();
                    }
                    if self.sleeping() {
                        return Ok(Exit::Sleep);
                    }
                }
                Err(Stop::Request(action)) => return Ok(Exit::Request(action)),
                Err(Stop::Horizon(action)) => return Ok(Exit::Horizon(action)),
                Err(Stop::Core(error)) => return Err(error),
                Err(Stop::Reset) => return Ok(Exit::Reset),
                Err(Stop::CommittedOwner) => return Ok(Exit::CommittedOwner),
            }
        }
    }

    fn exception_exit(&mut self) -> Result<Exit, Error> {
        let action = self
            .issued_action()
            .ok_or(Error::Internal("exception without physical entry work"))?;
        Ok(Exit::Exception(Request {
            action,
            admission: false,
            exception: self.accepted_vector.take(),
        }))
    }
    /// Resolve a stopped CPU without completing another physical effect.
    #[inline(always)]
    pub(crate) fn request(&mut self, interrupt: Option<u8>) -> Result<Request, Error> {
        let admission = self.boundary();
        let action = self.next_inner(|| interrupt)?;
        Ok(Request {
            action,
            admission,
            exception: self.accepted_vector.take(),
        })
    }
    #[cfg(test)]
    fn complete_request(&mut self, value: u16, interrupt: Option<u8>) -> Result<Request, Error> {
        self.complete(value)?;
        self.request(interrupt)
    }
    #[inline(always)]
    fn execute_phase<B: Bus>(&mut self, bus: &mut B) -> Result<bool, Stop> {
        #[cfg(feature = "profile-work")]
        self.phase_dispatches.add(1);
        match self.phase {
            Phase::ResetVector => {
                let value = bus.read(0, Width::Word, false)?;
                self.phase = Phase::EntryWait { target: value };
            }
            Phase::Prefetch { address, retain } => {
                let value = bus.read(address & !1, Width::Word, true)?;
                if retain {
                    self.prefetch = Some((address, value));
                }
                if matches!(self.continuation, Phase::ExceptionPc { .. }) {
                    self.phase = Phase::Delay(2);
                } else {
                    self.continue_execution()?;
                }
            }
            Phase::Delay(states) => {
                bus.idle(states)?;
                self.continue_execution()?;
            }
            Phase::BranchTarget { target, take } => {
                let value = bus.read(target & !1, Width::Word, true)?;
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
                let value = bus.read(target & !1, Width::Word, true)?;
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
            Phase::EntryWait { target } => {
                bus.idle(2)?;
                self.phase = Phase::EntryTarget { target };
            }
            Phase::EntryTarget { target } => {
                let value = bus.read(target & !1, Width::Word, true)?;
                self.registers.pc = target & !1;
                self.prefetch = Some((target & !1, value));
                self.phase = Phase::Boundary;
            }
            Phase::Fetch => {
                let value = bus.read(self.registers.pc & !1, Width::Word, true)?;
                if let Some(instruction) = self.accept_fetched(value)? {
                    if B::RUN_AHEAD {
                        self.run_instruction(instruction, bus, true)?;
                    } else {
                        self.prepare(instruction)?;
                    }
                }
            }
            Phase::Memory(t) => {
                let value = t.transact(bus)?;
                if let Some(t) = self.accept_transfer(t, value) {
                    self.phase = Phase::Memory(t);
                }
            }
            Phase::BitRead { address, op, bit } => {
                let value = bus.read(address, Width::Byte, false)?;
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
            Phase::BitWrite { address, value } => {
                bus.write(address, Width::Byte, u16::from(value), false)?;
                self.finish();
            }
            Phase::IndirectJump { address, call } => {
                let value = bus.read(address & !1, Width::Word, false)?;
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
                address,
                return_pc,
            } => {
                bus.write(address & !1, Width::Word, return_pc, false)?;
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
                bus.idle(2)?;
                self.phase = Phase::JumpTarget {
                    target,
                    call,
                    return_pc: self.registers.pc,
                }
            }
            Phase::ReturnPc => {
                let value = bus.read(self.registers.sp() & !1, Width::Word, false)?;
                self.registers.move_sp(2);
                self.jump(value, false);
            }
            Phase::ReturnCcr => {
                let value = bus.read(self.registers.sp() & !1, Width::Word, false)?;
                self.registers.move_sp(2);
                self.phase = Phase::ReturnExceptionPc {
                    ccr: (value >> 8) as u8,
                };
            }
            Phase::ReturnExceptionPc { ccr } => {
                let value = bus.read(self.registers.sp() & !1, Width::Word, false)?;
                self.registers.move_sp(2);
                self.registers.ccr = ccr;
                // RTE is not one of §3.8.5's CCR-writing deferral instructions.
                self.jump(value, false);
            }
            Phase::ExceptionPc { vector, pc, ccr } => {
                bus.write(self.registers.sp() & !1, Width::Word, pc, false)?;
                self.registers.move_sp(-2);
                self.phase = Phase::ExceptionCcr { vector, ccr };
            }
            Phase::ExceptionCcr { vector, ccr } => {
                bus.write(
                    self.registers.sp() & !1,
                    Width::Word,
                    u16::from(ccr) * 0x0101,
                    false,
                )?;
                self.phase = Phase::ExceptionVector { vector };
            }
            Phase::ExceptionVector { vector } => {
                let value = bus.read(u16::from(vector) * 2, Width::Word, false)?;
                self.phase = Phase::EntryWait { target: value };
            }
            Phase::MulDiv {
                size,
                dst,
                value: result,
                flags,
                states,
            } => {
                bus.idle(states)?;
                self.registers.write(size, dst, result);
                self.registers.ccr = flags;
                self.finish();
            }
            Phase::Copy {
                word_count,
                stage,
                value: latched,
            } => {
                let value = match stage {
                    0 | 4 => bus.read(self.registers.er[5] as u16, Width::Byte, false)?,
                    1 => bus.read(self.registers.er[6] as u16, Width::Byte, false)?,
                    3 => {
                        bus.write(
                            self.registers.er[6] as u16,
                            Width::Byte,
                            u16::from(latched),
                            false,
                        )?;
                        0
                    }
                    _ => return Ok(false),
                };
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
            _ => return Ok(false),
        }
        Ok(true)
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
    fn accept_transfer(&mut self, mut t: Transfer, value: u16) -> Option<Transfer> {
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
            return Some(t);
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
        None
    }
    /// NEXT and the instruction's register/operand continuation have one source.
    /// Single-effect diagnostic adapters enter after NEXT; interval execution
    /// proceeds directly and publishes the exact hardware phase only on a fence.
    #[inline]
    fn run_instruction<B: Bus>(
        &mut self,
        instruction: Instruction,
        bus: &mut B,
        next_due: bool,
    ) -> Result<bool, Stop> {
        // In this prototype the work witness includes both physical-phase and
        // instruction-family selection; the predecessor counts phases only.
        #[cfg(feature = "profile-work")]
        self.phase_dispatches.add(1);
        match instruction {
            Instruction::Nop => {
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
                self.finish();
            }
            Instruction::Binary { op, size, dst, src } => {
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
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
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
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
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
                let (r, f) =
                    alu::shift(op, size, self.registers.read(size, dst), self.registers.ccr);
                self.registers.write(size, dst, r);
                self.registers.ccr = f;
                self.finish();
            }
            Instruction::Quick { dst, delta } => {
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
                self.registers.er[usize::from(dst)] =
                    self.registers.er[usize::from(dst)].wrapping_add(i32::from(delta) as u32);
                self.finish();
            }
            Instruction::Memory {
                size,
                reg,
                address,
                store,
                ccr,
            } => {
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
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
                let transfer = Transfer {
                    address,
                    size,
                    register: reg,
                    store,
                    ccr,
                    absolute8,
                    value: if store { value } else { 0 },
                    done: 0,
                    post,
                };
                if updating {
                    if B::RUN_AHEAD {
                        if let Err(stop) = bus.idle(2) {
                            self.delay_then(2, Phase::Memory(transfer));
                            return Err(stop);
                        }
                    } else {
                        self.delay_then(2, Phase::Memory(transfer));
                        return Ok(true);
                    }
                }
                if B::RUN_AHEAD {
                    let mut t = transfer;
                    loop {
                        let value = match t.transact(bus) {
                            Ok(value) => value,
                            Err(stop) => {
                                self.phase = Phase::Memory(t);
                                return Err(stop);
                            }
                        };
                        match self.accept_transfer(t, value) {
                            Some(next) => t = next,
                            None => break,
                        }
                    }
                } else {
                    self.phase = Phase::Memory(transfer);
                }
            }
            Instruction::Ccr { op, source } => {
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
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
                if next_due {
                    self.instruction_next(instruction, bus)?;
                }
                self.registers
                    .write(Size::Byte, r, u32::from(self.registers.ccr));
                self.finish();
            }
            other => {
                if next_due {
                    self.prepare(other)?;
                } else {
                    self.begin_remaining(other)?;
                }
            }
        }
        Ok(true)
    }
    #[inline]
    fn instruction_next(
        &mut self,
        instruction: Instruction,
        bus: &mut impl Bus,
    ) -> Result<(), Stop> {
        let address = self.registers.pc & !1;
        let word = match bus.read(address, Width::Word, true) {
            Ok(word) => word,
            Err(stop) => {
                self.prefetch_then(address, true, Phase::Execute(instruction));
                return Err(stop);
            }
        };
        self.prefetch = Some((address, word));
        Ok(())
    }
    fn begin(&mut self, instruction: Instruction) -> Result<(), Error> {
        match self.run_instruction(instruction, &mut Reply(0), false) {
            Ok(_) => Ok(()),
            Err(Stop::Core(error)) => Err(error),
            _ => Err(Error::Internal(
                "instruction continuation requested a physical effect",
            )),
        }
    }
    fn begin_remaining(&mut self, instruction: Instruction) -> Result<(), Error> {
        match instruction {
            Instruction::Sleep => {
                self.retired = self.retired.wrapping_add(1);
                self.phase = Phase::Sleeping;
            }
            Instruction::Bit { op, bit, target } => {
                let bit = match bit {
                    BitIndex::Reg(r) => self.registers.read(Size::Byte, r) as u8 & 7,
                    BitIndex::Imm(bit) => bit,
                };
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
            _ => return Err(Error::Internal("instruction outside its semantic family")),
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
    struct LimitedBus {
        words: Vec<(u16, u16)>,
        effects: Vec<Action>,
        remaining: usize,
        reset: bool,
        irq_after: Option<usize>,
    }
    impl LimitedBus {
        fn perform(&mut self, action: Action) -> Result<(), Stop> {
            if self.remaining == 0 {
                return Err(if self.reset {
                    Stop::Reset
                } else {
                    Stop::Horizon(action)
                });
            }
            self.remaining -= 1;
            self.effects.push(action);
            Ok(())
        }
    }
    impl Bus for LimitedBus {
        const RUN_AHEAD: bool = true;
        fn read(&mut self, address: u16, width: Width, fetch: bool) -> Result<u16, Stop> {
            self.perform(Action::Read {
                address,
                width,
                fetch,
            })?;
            Ok(self
                .words
                .iter()
                .find(|(at, _)| *at == address)
                .map_or(0, |(_, value)| *value))
        }
        fn write(
            &mut self,
            address: u16,
            width: Width,
            value: u16,
            mov_byte: bool,
        ) -> Result<(), Stop> {
            self.perform(Action::Write {
                address,
                width,
                value,
                mov_byte,
            })
        }
        fn idle(&mut self, states: u32) -> Result<(), Stop> {
            self.perform(Action::Idle(states))
        }
    }
    impl IntervalBus for LimitedBus {
        fn interrupt(&self) -> Option<u8> {
            self.irq_after
                .filter(|after| self.effects.len() >= *after)
                .map(|_| 7)
        }
        fn instruction_boundary(&mut self) {}
    }
    #[test]
    fn direct_long_store_retains_first_lane_and_restores_without_reissuing_it() {
        for reset in [false, true] {
            let mut cpu = Cpu::new(0x100);
            cpu.registers.er[2] = 0x1234_ff00;
            let mut bus = LimitedBus {
                words: vec![(0x100, 0x0100), (0x102, 0x6da2)],
                effects: Vec::new(),
                remaining: 5,
                reset,
                irq_after: None,
            };
            let exit = cpu.run_interval(&mut bus).unwrap();
            assert!(if reset {
                matches!(exit, Exit::Reset)
            } else {
                matches!(exit, Exit::Horizon(_))
            });
            assert_eq!(
                bus.effects,
                vec![
                    Action::Read {
                        address: 0x100,
                        width: Width::Word,
                        fetch: true
                    },
                    Action::Read {
                        address: 0x102,
                        width: Width::Word,
                        fetch: true
                    },
                    Action::Read {
                        address: 0x104,
                        width: Width::Word,
                        fetch: true
                    },
                    Action::Idle(2),
                    Action::Write {
                        address: 0xfefc,
                        width: Width::Word,
                        value: 0x1234,
                        mov_byte: false
                    },
                ]
            );
            assert_eq!(cpu.registers.er[2], 0x1234_fefc);
            assert_eq!(cpu.retired, 0);
            let mut restored = cpu.save().unwrap().restore(false).unwrap();
            assert_eq!(
                restored.next(|| Some(7)).unwrap(),
                Action::Write {
                    address: 0xfefe,
                    width: Width::Word,
                    value: 0xfefc,
                    mov_byte: false,
                }
            );
            restored.complete(0).unwrap();
            assert_eq!(restored.retired, 1);
            assert_eq!(restored.accepted_vector, None);
        }
    }
    #[test]
    fn direct_next_fence_keeps_register_destination_and_actual_prefetched_bytes() {
        let mut cpu = Cpu::new(0x100);
        cpu.registers.er[0] = 0x1122_3344;
        let mut bus = LimitedBus {
            words: vec![(0x100, 0xf8ab), (0x102, 0x0000)],
            effects: Vec::new(),
            remaining: 1,
            reset: false,
            irq_after: None,
        };
        assert!(matches!(
            cpu.run_interval(&mut bus).unwrap(),
            Exit::Horizon(Action::Read { address: 0x102, .. })
        ));
        assert_eq!(cpu.registers.er[0], 0x1122_3344);
        let mut restored = cpu.save().unwrap().restore(false).unwrap();
        restored.complete(0).unwrap();
        assert_eq!(restored.registers.er[0], 0x1122_33ab);
        assert_eq!(restored.retired, 1);
        // Change code behind the hardware fetch: the retained NOP still executes.
        let mut bus = LimitedBus {
            words: vec![(0x102, 0xffff)],
            effects: Vec::new(),
            remaining: 1,
            reset: false,
            irq_after: None,
        };
        assert!(matches!(
            restored.run_interval(&mut bus).unwrap(),
            Exit::Horizon(_)
        ));
        assert_eq!(restored.retired, 2);
        assert_eq!(
            bus.effects,
            vec![Action::Read {
                address: 0x104,
                width: Width::Word,
                fetch: true
            }]
        );
    }
    #[test]
    fn invalid_retained_word_preserves_fault_prefix_and_pc_after_direct_retirement() {
        let mut cpu = Cpu::new(0x100);
        let mut bus = LimitedBus {
            words: vec![(0x100, 0xf801), (0x102, 0x0001)],
            effects: Vec::new(),
            remaining: 4,
            reset: false,
            irq_after: None,
        };
        assert!(matches!(
            cpu.run_interval(&mut bus),
            Err(Error::Decode {
                pc: 0x102,
                count: 1,
                words: [0x0001, 0, 0, 0, 0]
            })
        ));
        assert_eq!(cpu.registers.er[0], 1);
        assert_eq!(cpu.registers.pc, 0x104);
        assert_eq!(cpu.retired, 1);
        assert!(cpu.save().unwrap().restore(true).is_ok());
        assert_eq!(bus.effects.len(), 2);
    }
    #[test]
    fn direct_ccr_write_defers_offered_nmi_for_one_intervening_instruction() {
        let mut cpu = Cpu::new(0x100);
        cpu.registers.er[7] = 0xff7c;
        let mut bus = LimitedBus {
            words: vec![(0x100, 0x0700), (0x102, 0xf801)],
            effects: Vec::new(),
            remaining: 5,
            reset: false,
            irq_after: Some(2),
        };
        let Exit::Exception(request) = cpu.run_interval(&mut bus).unwrap() else {
            panic!("NMI was not admitted")
        };
        assert_eq!(request.exception, Some(7));
        assert_eq!(
            request.action,
            Action::Read {
                address: 0x106,
                width: Width::Word,
                fetch: true
            }
        );
        assert_eq!(cpu.retired, 2);
        assert_eq!(cpu.registers.er[0], 1);
        assert_eq!(cpu.registers.pc, 0x104);
        assert_eq!(
            bus.effects,
            vec![
                Action::Read {
                    address: 0x100,
                    width: Width::Word,
                    fetch: true
                },
                Action::Read {
                    address: 0x102,
                    width: Width::Word,
                    fetch: true
                },
                Action::Read {
                    address: 0x104,
                    width: Width::Word,
                    fetch: true
                },
            ]
        );
    }
    #[test]
    fn projection_preserves_the_operand_phase_and_reply_does_not_admit_nmi() {
        let mut cpu = Cpu::new(0x100);
        cpu.registers.er[0] = 0x300;
        cpu.registers.er[1] = 0x1122_3344;
        cpu.registers.er[7] = 0xff7c;
        cpu.next(|| None).unwrap();
        cpu.complete(0x6901).unwrap(); // MOV.W @ER0,R1.
        cpu.next(|| None).unwrap();
        cpu.complete(0).unwrap(); // Retained NEXT.
        let before = cpu.clone();
        assert_eq!(
            cpu.issued_action(),
            Some(Action::Read {
                address: 0x300,
                width: Width::Word,
                fetch: false,
            })
        );
        assert_eq!(cpu, before);
        cpu.complete(0xabcd).unwrap();
        assert_eq!(cpu.registers.er[1], 0x1122_abcd);
        assert_eq!(cpu.retired, 1);
        assert_eq!(cpu.accepted_vector, None);
        assert_eq!(
            cpu.next(|| Some(7)).unwrap(),
            Action::Read {
                address: 0x104,
                width: Width::Word,
                fetch: true,
            }
        );
        assert_eq!(cpu.accepted_vector, Some(7));
    }
    #[test]
    fn request_reports_exception_once_before_the_physical_entry_sequence() {
        let mut cpu = Cpu::new(0x1234);
        cpu.registers.er[7] = 0xff7c;
        cpu.registers.ccr = 0x21;
        let first = cpu.request(Some(25)).unwrap();
        assert!(first.admission);
        assert_eq!(first.exception, Some(25));
        assert_eq!(
            first.action,
            Action::Read {
                address: 0x1236,
                width: Width::Word,
                fetch: true,
            }
        );
        let repeated = cpu.request(Some(7)).unwrap();
        assert_eq!(repeated.action, first.action);
        assert!(!repeated.admission);
        assert_eq!(repeated.exception, None);
        for (value, action) in [
            (0, Action::Idle(2)),
            (
                0,
                Action::Write {
                    address: 0xff7a,
                    width: Width::Word,
                    value: 0x1234,
                    mov_byte: false,
                },
            ),
            (
                0,
                Action::Write {
                    address: 0xff78,
                    width: Width::Word,
                    value: 0x2121,
                    mov_byte: false,
                },
            ),
            (
                0,
                Action::Read {
                    address: 50,
                    width: Width::Word,
                    fetch: false,
                },
            ),
            (0x200, Action::Idle(2)),
            (
                0,
                Action::Read {
                    address: 0x200,
                    width: Width::Word,
                    fetch: true,
                },
            ),
        ] {
            let next = cpu.complete_request(value, None).unwrap();
            assert_eq!(next.action, action);
            assert!(!next.admission);
            assert_eq!(next.exception, None);
        }
        let handler = cpu.complete_request(0x0000, None).unwrap();
        assert!(handler.admission);
        assert_eq!(handler.exception, None);
        assert_eq!(cpu.registers.pc, 0x202);
        assert_eq!(cpu.registers.ccr, 0xa1);
    }
    #[test]
    fn copy_pair_admission_is_distinct_from_an_instruction_boundary() {
        for word_count in [false, true] {
            let mut cpu = Cpu::new(0x100);
            cpu.registers.er[4] = 1;
            cpu.registers.er[5] = 0x300;
            cpu.registers.er[6] = 0x400;
            cpu.registers.er[7] = 0xff7c;
            assert!(cpu.request(None).unwrap().admission);
            for word in [if word_count { 0x7bd4 } else { 0x7b5c }, 0x598f, 0] {
                let next = cpu.complete_request(word, Some(7)).unwrap();
                assert!(!next.admission);
                assert_eq!(next.exception, None);
            }
            let pair = cpu.complete_request(0, Some(7)).unwrap();
            assert!(!pair.admission);
            assert_eq!(pair.exception, word_count.then_some(7));
            assert_eq!(
                pair.action,
                Action::Read {
                    address: if word_count { 0x106 } else { 0x300 },
                    width: if word_count { Width::Word } else { Width::Byte },
                    fetch: word_count,
                }
            );
            assert_eq!(cpu.registers.er[4], 1);
            assert_eq!(cpu.registers.er[5], 0x300);
            assert_eq!(cpu.registers.er[6], 0x400);
        }
    }
    #[test]
    fn request_fault_retains_the_invalid_fetch_and_pending_notification() {
        let mut cpu = Cpu::new(0x100);
        cpu.registers.er[7] = 0xff7c;
        // Run entry through the diagnostic interface, which leaves the accepted
        // vector for integration to acknowledge. Fetch an invalid handler word.
        cpu.next(|| Some(7)).unwrap();
        for value in [0, 0, 0, 0, 0x200, 0] {
            cpu.complete(value).unwrap();
            cpu.next(|| None).unwrap();
        }
        cpu.complete(1).unwrap();
        assert!(matches!(
            cpu.request(None),
            Err(Error::Decode {
                pc: 0x200,
                count: 1,
                ..
            })
        ));
        assert_eq!(cpu.registers.pc, 0x202);
        assert_eq!(cpu.words[0], 1);
        assert_eq!(cpu.phase, Phase::Fetch);
        assert_eq!(cpu.accepted_vector, Some(7));
    }
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
            match cpu.next(|| None).unwrap() {
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
            cpu.next(|| Some(25)).unwrap(),
            Action::Read {
                address: 0x1236,
                width: Width::Word,
                fetch: true,
            }
        );
        cpu.complete(0).unwrap();
        assert_eq!(cpu.next(|| None).unwrap(), Action::Idle(2));
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(|| None).unwrap(),
            Action::Write {
                address: 0xff7a,
                width: Width::Word,
                value: 0x1234,
                mov_byte: false
            }
        );
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(|| None).unwrap(),
            Action::Write {
                address: 0xff78,
                width: Width::Word,
                value: 0x2121,
                mov_byte: false
            }
        );
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(|| None).unwrap(),
            Action::Read {
                address: 50,
                width: Width::Word,
                fetch: false,
            }
        );
        cpu.complete(0x200).unwrap();
        assert_eq!(cpu.next(|| None).unwrap(), Action::Idle(2));
        cpu.complete(0).unwrap();
        assert_eq!(
            cpu.next(|| None).unwrap(),
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
