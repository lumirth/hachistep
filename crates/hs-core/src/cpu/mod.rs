//! One resumable H8/300H executor.
//!
//! Each unit of CPU work — an instruction, an exception or reset entry, or one
//! EEPMOV transfer pair — is authored once, as straight-line code over a [`Bus`].
//! Work can stop before any physical access. The CPU then keeps the results of
//! the accesses that already completed and returns to the registers it had when
//! the work began. Resuming executes the work again and replays those completed
//! accesses instead of repeating them, so a suspension never changes which
//! accesses happen, their order, or their timing.
pub mod alu;
pub mod decode;
pub(crate) mod direct;
pub(crate) mod execution;
mod memo;
mod repetition;
pub(crate) mod state;
use crate::error::Error;
use alu::{C, H, I, N, Z};
use decode::{Address, Alu, BitIndex, CcrOp, Decode, Instruction, Jump, Size, Source, Target};
use execution::{Bus, Exit, IntervalBus, Projection, Stop};
use memo::{DecodeMemo, Prelude};
use repetition::Repetition;

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
        origin: WriteOrigin,
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
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
pub enum WriteOrigin {
    Other = 0,
    MovByte = 1,
    MovByteAbs8 { pc_bit1: bool } = 2,
}
impl WriteOrigin {
    pub fn is_mov(self) -> bool {
        self != Self::Other
    }
}
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registers {
    pub er: [u32; 8],
    pub pc: u16,
    pub ccr: u8,
}
impl Registers {
    #[inline(always)]
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
    #[inline(always)]
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
}

/// What the CPU is doing between physical accesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// The previous work retired; admission and the next fetch have not occurred.
    Boundary,
    Sleeping,
    Busy(Work),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Work {
    /// Vector read and pipeline fill after reset.
    Reset,
    Instruction,
    /// An admitted exception. `pc` and `ccr` are the values being stacked.
    Exception {
        vector: u8,
        entry: Entry,
        pc: u16,
        ccr: u8,
    },
    /// The remaining EEPMOV transfer pairs; each pair is its own unit of work.
    Copy {
        word_count: bool,
    },
}
/// The operation that precedes exception stacking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// An instruction boundary discards a fetch of the word after NEXT and
    /// spends two internal states.
    Fetch,
    /// TRAPA has already fetched NEXT; two internal states remain.
    Trap,
    /// A sleeping CPU substitutes four internal states for the fetch.
    Sleep,
}
/// Results of the physical operations the current work already completed.
/// Internal states record zero; EEPMOV.W pair admission records its decision.
pub(crate) const COMPLETED: usize = 12;
#[derive(Clone, Copy, Debug)]
struct Completed {
    values: [u16; COMPLETED],
    count: u8,
    replayed: u8,
}
/// Slots beyond `count` are stale, not state.
impl PartialEq for Completed {
    fn eq(&self, other: &Self) -> bool {
        (self.count, self.replayed) == (other.count, other.replayed)
            && self.values[..usize::from(self.count)] == other.values[..usize::from(other.count)]
    }
}
impl Eq for Completed {}
impl Completed {
    #[inline(always)]
    fn replay(&mut self) -> Option<u16> {
        if self.replayed < self.count {
            let value = self.values[usize::from(self.replayed)];
            self.replayed += 1;
            return Some(value);
        }
        None
    }
    #[inline(always)]
    fn record(&mut self, value: u16) -> Result<(), Error> {
        let slot = self
            .values
            .get_mut(usize::from(self.count))
            .ok_or(Error::Internal("CPU work exceeded its access record"))?;
        *slot = value;
        self.count += 1;
        self.replayed = self.count;
        Ok(())
    }
}
/// The retained NEXT fetch, packed so that it is always stored and loaded
/// whole. An odd address is never retained and marks no prefetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Prefetch(u32);
impl Prefetch {
    const NONE: Self = Self(1 << 16);
    #[inline(always)]
    fn new(address: u16, word: u16) -> Self {
        Self(u32::from(address) << 16 | u32::from(word))
    }
    fn from_saved(saved: Option<(u16, u16)>) -> Self {
        saved.map_or(Self::NONE, |(address, word)| Self::new(address, word))
    }
    #[inline(always)]
    fn get(self) -> Option<(u16, u16)> {
        (self.0 & 1 << 16 == 0).then_some(((self.0 >> 16) as u16, self.0 as u16))
    }
}
/// Architectural state when the current work began, restored on suspension.
/// General registers are journaled on their first write in the work, so
/// beginning work copies only the scalar state.
#[derive(Clone, Copy, Debug)]
struct Origin {
    pc: u16,
    ccr: u8,
    prefetch: Prefetch,
    interrupt_delay: u8,
    /// Values of the general registers marked in `written`.
    er: [u32; 8],
    written: u8,
}
/// Journal slots of unwritten registers are stale, not state.
impl PartialEq for Origin {
    fn eq(&self, other: &Self) -> bool {
        (
            self.pc,
            self.ccr,
            self.prefetch,
            self.interrupt_delay,
            self.written,
        ) == (
            other.pc,
            other.ccr,
            other.prefetch,
            other.interrupt_delay,
            other.written,
        ) && (0..8).all(|i| self.written & 1 << i == 0 || self.er[i] == other.er[i])
    }
}
impl Eq for Origin {}
impl Origin {
    fn new(cpu_registers: &Registers, prefetch: Prefetch, interrupt_delay: u8) -> Self {
        Self {
            pc: cpu_registers.pc,
            ccr: cpu_registers.ccr,
            prefetch,
            interrupt_delay,
            er: [0; 8],
            written: 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub registers: Registers,
    state: State,
    prefetch: Prefetch,
    interrupt_delay: u8,
    instruction_pc: u16,
    accepted_vector: Option<u8>,
    origin: Origin,
    completed: Completed,
    memo: DecodeMemo,
    code: direct::Code,
    repetition: Repetition,
    pub retired: u64,
    pub interrupt_entries: u64,
}
impl Cpu {
    /// A CPU entering reset at power-up.
    pub(crate) fn new() -> Self {
        let mut cpu = Self::bare();
        cpu.prepare();
        cpu.reset();
        cpu
    }
    /// Architectural work without disposable decode/execution storage.
    fn bare() -> Self {
        Self {
            registers: Registers {
                er: [0; 8],
                pc: 0,
                ccr: I,
            },
            state: State::Boundary,
            prefetch: Prefetch::NONE,
            interrupt_delay: 0,
            instruction_pc: 0,
            accepted_vector: None,
            origin: Origin::new(
                &Registers {
                    er: [0; 8],
                    pc: 0,
                    ccr: I,
                },
                Prefetch::NONE,
                0,
            ),
            completed: Completed {
                values: [0; COMPLETED],
                count: 0,
                replayed: 0,
            },
            memo: DecodeMemo::empty(),
            code: direct::Code::empty(),
            repetition: Repetition::new(),
            retired: 0,
            interrupt_entries: 0,
        }
    }
    /// Reset entry performs the vector read and pipeline fill on the bus.
    /// The model initializes general registers to zero. Diagnostic totals
    /// continue across resets.
    /// Allocate what running needs, so a copy runs without allocating.
    pub(crate) fn prepare(&mut self) {
        self.memo.prepare();
        self.code.prepare();
    }
    pub(crate) fn reset(&mut self) {
        self.registers = Registers {
            er: [0; 8],
            pc: 0,
            ccr: I,
        };
        // The first reset instruction runs even with NMI pending.
        self.interrupt_delay = 1;
        self.prefetch = Prefetch::NONE;
        self.instruction_pc = 0;
        self.accepted_vector = None;
        self.completed.count = 0;
        self.completed.replayed = 0;
        self.begin_work(Work::Reset);
    }
    pub fn sleeping(&self) -> bool {
        self.state == State::Sleeping
    }
    /// An admission point: an instruction boundary or SLEEP.
    pub fn boundary(&self) -> bool {
        matches!(self.state, State::Boundary | State::Sleeping)
    }
    pub fn instruction_pc(&self) -> u16 {
        self.instruction_pc
    }
    pub fn phase_name(&self) -> &'static str {
        match self.state {
            State::Boundary => "boundary",
            State::Sleeping => "sleep",
            State::Busy(Work::Reset) => "reset-entry",
            State::Busy(Work::Instruction) => "instruction",
            State::Busy(Work::Exception { .. }) => "exception-entry",
            State::Busy(Work::Copy { .. }) => "eepmov",
        }
    }
    pub(crate) fn direct_transition(&mut self) -> Result<(), Error> {
        if !self.sleeping() {
            return Err(Error::Internal("direct transition outside SLEEP"));
        }
        self.enter_exception(13, Entry::Sleep);
        Ok(())
    }

    // Work lifecycle.

    fn begin_work(&mut self, work: Work) {
        self.state = State::Busy(work);
        self.completed.count = 0;
        self.completed.replayed = 0;
        // Journal slots keep stale values; only `written` marks valid ones.
        let origin = &mut self.origin;
        origin.pc = self.registers.pc;
        origin.ccr = self.registers.ccr;
        origin.prefetch = self.prefetch;
        origin.interrupt_delay = self.interrupt_delay;
        origin.written = 0;
    }
    fn end_work(&mut self, next: State) -> Result<(), Stop> {
        if self.completed.replayed != self.completed.count {
            return Err(Error::Internal("CPU work finished before its recorded accesses").into());
        }
        self.state = next;
        self.origin.written = 0;
        self.completed.count = 0;
        self.completed.replayed = 0;
        Ok(())
    }
    fn suspend(&mut self) {
        self.repetition.branched = false;
        let origin = &mut self.origin;
        for (i, er) in self.registers.er.iter_mut().enumerate() {
            if origin.written & 1 << i != 0 {
                *er = origin.er[i];
            }
        }
        origin.written = 0;
        self.registers.pc = origin.pc;
        self.registers.ccr = origin.ccr;
        self.prefetch = origin.prefetch;
        self.interrupt_delay = origin.interrupt_delay;
    }
    /// The general register `index` for writing, journaled for suspension.
    #[inline(always)]
    fn er_mut(&mut self, index: u8) -> &mut u32 {
        let i = usize::from(index & 7);
        if self.origin.written & 1 << i == 0 {
            self.origin.written |= 1 << i;
            self.origin.er[i] = self.registers.er[i];
        }
        &mut self.registers.er[i]
    }
    #[inline(always)]
    fn write_register(&mut self, size: Size, field: u8, value: u32) {
        self.er_mut(field);
        self.registers.write(size, field, value);
    }
    #[inline(always)]
    fn move_sp(&mut self, delta: i32) {
        let sp = self.er_mut(7);
        *sp = sp.wrapping_add(delta as u32);
    }
    fn retire(&mut self) -> Result<(), Stop> {
        self.retired = self.retired.wrapping_add(1);
        self.end_work(State::Boundary)
    }
    /// Sample the offered request at an admission point; otherwise begin the
    /// next instruction. `interrupt` is consulted only when admission is open.
    #[inline(always)]
    fn admit(&mut self, interrupt: impl FnOnce() -> Option<u8>) {
        if self.interrupt_delay == 0 {
            if let Some(vector) = interrupt() {
                if vector == 7 || self.registers.ccr & I == 0 {
                    let entry = if self.sleeping() {
                        Entry::Sleep
                    } else {
                        Entry::Fetch
                    };
                    self.enter_exception(vector, entry);
                    return;
                }
            }
        }
        if self.sleeping() {
            return;
        }
        self.interrupt_delay = self.interrupt_delay.saturating_sub(1);
        self.instruction_pc = self.registers.pc;
        self.begin_work(Work::Instruction);
    }
    fn enter_exception(&mut self, vector: u8, entry: Entry) {
        let pc = self.registers.pc;
        let ccr = self.registers.ccr;
        self.registers.ccr |= I;
        self.interrupt_entries = self.interrupt_entries.wrapping_add(1);
        self.accepted_vector = Some(vector);
        self.prefetch = Prefetch::NONE;
        self.move_sp(-2);
        self.begin_work(Work::Exception {
            vector,
            entry,
            pc,
            ccr,
        });
    }
    /// Execute the current work from its origin. A stop suspends it.
    #[inline(always)]
    fn execute(&mut self, bus: &mut impl Bus) -> Result<(), Stop> {
        let State::Busy(work) = self.state else {
            return Ok(());
        };
        self.completed.replayed = 0;
        let result = match work {
            Work::Instruction => self.instruction(bus),
            Work::Exception {
                vector,
                entry,
                pc,
                ccr,
            } => self.exception(bus, vector, entry, pc, ccr),
            Work::Reset => {
                let target = self.read(bus, 0, Width::Word, false);
                target.and_then(|target| self.enter_target(bus, target))
            }
            Work::Copy { word_count } => self.copy_pair(bus, word_count),
        };
        if result.is_err() {
            self.suspend();
        }
        result
    }

    // Interfaces used by Machine.

    /// Execute work over the bus until the horizon, a clock fence, SLEEP or a
    /// reset. Every access is performed by the bus at its own time.
    #[inline]
    pub(crate) fn run_interval(&mut self, bus: &mut impl IntervalBus) -> Result<Exit, Error> {
        self.repetition.forget();
        loop {
            bus.direct(|memory, run| self.run_direct(memory, run))?;
            if self.boundary() {
                if !bus.proceed() {
                    return Ok(Exit::Fence);
                }
                self.admit(|| bus.interrupt());
                bus.instruction_boundary();
                #[cfg(feature = "trace")]
                bus.instruction(self.instruction_pc);
                if self.sleeping() {
                    return Ok(Exit::Sleep);
                }
            }
            if let Some(vector) = self.accepted_vector.take() {
                if let Err(stop) = bus.accept(vector) {
                    return Self::exit(stop);
                }
            }
            if let Err(stop) = self.execute(bus) {
                return Self::exit(stop);
            }
            if self.repetition.branched {
                self.recognize(bus);
            }
        }
    }
    /// A taken backward branch retired: account for repetitions of the
    /// iteration it closed when that iteration provably repeats.
    #[inline(never)]
    fn recognize(&mut self, bus: &mut impl Bus) {
        self.repetition.branched = false;
        let Some(mark) = bus.mark() else {
            self.repetition.forget();
            return;
        };
        let Some((since, retired)) = self.repetition.arrive(
            &self.registers,
            self.prefetch,
            self.interrupt_delay,
            mark,
            self.retired,
        ) else {
            return;
        };
        let repetitions = bus.repeat(&since);
        if repetitions != 0 {
            self.retired = self.retired.wrapping_add(repetitions.wrapping_mul(retired));
            if let Some(mark) = bus.mark() {
                self.repetition.advanced(mark, self.retired);
            }
        }
    }
    fn exit(stop: Stop) -> Result<Exit, Error> {
        match stop {
            Stop::Horizon(action) => Ok(Exit::Horizon(action)),
            Stop::Fence => Ok(Exit::Fence),
            Stop::Reset => Ok(Exit::Reset),
            Stop::Core(error) => Err(error),
            Stop::Request(_) | Stop::Sample => {
                Err(Error::Internal("interval bus deferred an access"))
            }
        }
    }
    /// Resolve a stopped CPU without completing another physical effect.
    pub(crate) fn request(&mut self, interrupt: Option<u8>) -> Result<Request, Error> {
        let admission = self.boundary();
        let mut bus = Projection {
            interrupt,
            admit: true,
        };
        loop {
            if self.boundary() {
                self.admit(|| interrupt);
                if self.sleeping() {
                    return Ok(Request {
                        action: Action::Sleep,
                        admission,
                        exception: self.accepted_vector.take(),
                    });
                }
            }
            match self.execute(&mut bus) {
                Ok(()) => {}
                Err(Stop::Request(action)) => {
                    return Ok(Request {
                        action,
                        admission,
                        exception: self.accepted_vector.take(),
                    })
                }
                Err(Stop::Core(error)) => return Err(error),
                Err(_) => return Err(Error::Internal("CPU projection without an action")),
            }
        }
    }
    /// The next physical operation of unfinished work, without changing state.
    pub(crate) fn issued_action(&self) -> Option<Action> {
        let mut probe = self.clone();
        match probe.execute(&mut Projection {
            interrupt: None,
            admit: false,
        }) {
            Err(Stop::Request(action)) => Some(action),
            _ => None,
        }
    }

    // Physical operations. Completed results are replayed, never repeated.

    #[inline(always)]
    fn read<B: Bus>(
        &mut self,
        bus: &mut B,
        address: u16,
        width: Width,
        fetch: bool,
    ) -> Result<u16, Stop> {
        if let Some(value) = self.completed.replay() {
            return Ok(value);
        }
        let value = bus.read(address, width, fetch)?;
        self.completed.record(value)?;
        Ok(value)
    }
    #[inline(always)]
    fn write<B: Bus>(
        &mut self,
        bus: &mut B,
        address: u16,
        width: Width,
        value: u16,
        origin: WriteOrigin,
    ) -> Result<(), Stop> {
        if self.completed.replay().is_some() {
            return Ok(());
        }
        bus.write(address, width, value, origin)?;
        self.completed.record(0)?;
        Ok(())
    }
    #[inline(always)]
    fn idle<B: Bus>(&mut self, bus: &mut B, states: u32) -> Result<(), Stop> {
        if self.completed.replay().is_some() {
            return Ok(());
        }
        bus.idle(states)?;
        self.completed.record(0)?;
        Ok(())
    }
    /// REJ09B0152-0300 §3.8.6: EEPMOV.W accepts NMI between transfer pairs.
    fn nmi_between_pairs<B: Bus>(&mut self, bus: &mut B) -> Result<bool, Stop> {
        if let Some(value) = self.completed.replay() {
            return Ok(value != 0);
        }
        let nmi = bus.sample()? == Some(7);
        self.completed.record(u16::from(nmi))?;
        Ok(nmi)
    }
    #[inline(always)]
    fn fetch(&mut self, bus: &mut impl Bus) -> Result<u16, Stop> {
        let word = self.read(bus, self.registers.pc & !1, Width::Word, true)?;
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(word)
    }
    /// REJ09B0213-0300 §2.8: NEXT is a real bus fetch, retained for the next
    /// instruction unless the instruction changes flow.
    #[inline(always)]
    fn fetch_next(&mut self, bus: &mut impl Bus, retain: bool) -> Result<(), Stop> {
        let address = self.registers.pc & !1;
        let word = self.read(bus, address, Width::Word, true)?;
        if retain {
            self.prefetch = Prefetch::new(address, word);
        }
        Ok(())
    }

    // Work definitions.

    fn exception(
        &mut self,
        bus: &mut impl Bus,
        vector: u8,
        entry: Entry,
        pc: u16,
        ccr: u8,
    ) -> Result<(), Stop> {
        match entry {
            Entry::Fetch => {
                self.read(bus, pc.wrapping_add(2) & !1, Width::Word, true)?;
                self.idle(bus, 2)?;
            }
            Entry::Trap => self.idle(bus, 2)?,
            Entry::Sleep => self.idle(bus, 4)?,
        }
        self.write(
            bus,
            self.registers.sp() & !1,
            Width::Word,
            pc,
            WriteOrigin::Other,
        )?;
        self.move_sp(-2);
        self.write(
            bus,
            self.registers.sp() & !1,
            Width::Word,
            u16::from(ccr) * 0x0101,
            WriteOrigin::Other,
        )?;
        let target = self.read(bus, u16::from(vector) * 2, Width::Word, false)?;
        self.enter_target(bus, target)
    }
    fn enter_target(&mut self, bus: &mut impl Bus, target: u16) -> Result<(), Stop> {
        self.idle(bus, 2)?;
        let word = self.read(bus, target & !1, Width::Word, true)?;
        self.registers.pc = target & !1;
        self.prefetch = Prefetch::new(target & !1, word);
        self.end_work(State::Boundary)
    }
    #[inline(always)]
    fn instruction(&mut self, bus: &mut impl Bus) -> Result<(), Stop> {
        let pc = self.registers.pc;
        let prefetch = core::mem::replace(&mut self.prefetch, Prefetch::NONE);
        let first = match prefetch.get() {
            Some((address, word)) if address == pc => {
                self.registers.pc = self.registers.pc.wrapping_add(2);
                word
            }
            _ => self.fetch(bus)?,
        };
        let mut words = [first, 0, 0, 0, 0];
        let mut count = 1;
        // A remembered decoding applies while the fetched words match it.
        if let Some((expected, length, instruction, prelude)) = self.memo.lookup(pc, first) {
            let mut matched = true;
            while count < length {
                let word = self.fetch(bus)?;
                words[count] = word;
                matched = word == expected[count];
                count += 1;
                if !matched {
                    break;
                }
            }
            if matched {
                return self.decoded(bus, instruction, prelude, count);
            }
        }
        let instruction = loop {
            match decode::decode(&words[..count]) {
                Decode::Ready { instruction, words } => {
                    if usize::from(words) != count {
                        return Err(
                            Error::Internal("decoder consumed a different fetch count").into()
                        );
                    }
                    break instruction;
                }
                Decode::NeedWord if count < words.len() => {
                    words[count] = self.fetch(bus)?;
                    count += 1;
                }
                Decode::NeedWord => {
                    return Err(Error::Internal("decoder requested more than ten bytes").into())
                }
                Decode::Invalid => {
                    return Err(Error::Decode {
                        pc: self.instruction_pc,
                        words,
                        count: count as u8,
                    }
                    .into())
                }
            }
        };
        let prelude = Prelude::of(&instruction, count);
        self.memo
            .remember(pc, &words[..count], instruction, prelude);
        self.decoded(bus, instruction, prelude, count)
    }
    #[inline(always)]
    fn decoded(
        &mut self,
        bus: &mut impl Bus,
        instruction: Instruction,
        prelude: Prelude,
        count: usize,
    ) -> Result<(), Stop> {
        match prelude {
            Prelude::None => {}
            Prelude::Calculate => self.idle(bus, 2)?,
            Prelude::Fetch => self.fetch_next(bus, false)?,
            Prelude::Retain => self.fetch_next(bus, true)?,
        }
        self.execute_instruction(bus, instruction, count == 1)
    }
    #[inline(always)]
    fn execute_instruction(
        &mut self,
        bus: &mut impl Bus,
        instruction: Instruction,
        short: bool,
    ) -> Result<(), Stop> {
        match instruction {
            Instruction::Nop => {}
            Instruction::Sleep => {
                self.retired = self.retired.wrapping_add(1);
                return self.end_work(State::Sleeping);
            }
            Instruction::Binary { op, size, dst, src } => {
                let a = self.registers.read(size, dst);
                let b = self.source(size, src);
                let (r, f) = alu::binary(op, size, a, b, self.registers.ccr);
                if op != Alu::Cmp {
                    self.write_register(size, dst, r);
                }
                self.registers.ccr = f;
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
                self.write_register(size, dst, r);
                self.registers.ccr = f;
            }
            Instruction::Shift { op, size, dst } => {
                let (r, f) =
                    alu::shift(op, size, self.registers.read(size, dst), self.registers.ccr);
                self.write_register(size, dst, r);
                self.registers.ccr = f;
            }
            Instruction::Quick { dst, delta } => {
                let r = self.er_mut(dst);
                *r = r.wrapping_add(i32::from(delta) as u32);
            }
            Instruction::Bit { op, bit, target } => {
                let bit = match bit {
                    BitIndex::Reg(r) => self.registers.read(Size::Byte, r) as u8 & 7,
                    BitIndex::Imm(bit) => bit,
                };
                match target {
                    Target::Reg(r) => {
                        let value = self.registers.read(Size::Byte, r) as u8;
                        let (result, writes) = alu::bit(&mut self.registers.ccr, op, bit, value);
                        if writes {
                            self.write_register(Size::Byte, r, u32::from(result));
                        }
                    }
                    Target::Memory(address) => {
                        let (address, _) = self.target_address(address, Size::Byte);
                        let value = self.read(bus, address, Width::Byte, false)?;
                        let (result, writes) =
                            alu::bit(&mut self.registers.ccr, op, bit, value as u8);
                        self.fetch_next(bus, true)?;
                        if writes {
                            self.write(
                                bus,
                                address,
                                Width::Byte,
                                u16::from(result),
                                WriteOrigin::Other,
                            )?;
                        }
                    }
                }
            }
            Instruction::Memory {
                size,
                reg,
                address,
                store,
                ccr,
            } => self.transfer(bus, size, reg, address, store, ccr, short)?,
            Instruction::Branch {
                condition,
                displacement,
            } => {
                let taken = alu::condition(condition, self.registers.ccr);
                // Bcc8 reads both fallthrough and target, even when untaken.
                // Bcc16 instead fetches only the selected next instruction.
                let target = if !short && !taken {
                    self.registers.pc
                } else {
                    self.registers.pc.wrapping_add(displacement as u16)
                };
                let word = self.read(bus, target & !1, Width::Word, true)?;
                if taken || !short {
                    self.registers.pc = target & !1;
                    self.prefetch = Prefetch::new(target & !1, word);
                }
                self.repetition.branched = taken && target & !1 <= self.instruction_pc;
            }
            Instruction::BranchSubroutine(displacement) => {
                let target = self.registers.pc.wrapping_add(displacement as u16);
                return self.jump_target(bus, target, Some(self.registers.pc));
            }
            Instruction::Jump { target, call } => {
                let return_pc = call.then_some(self.registers.pc);
                match target {
                    Jump::Absolute(address) => {
                        self.idle(bus, 2)?;
                        return self.jump_target(bus, address, return_pc);
                    }
                    Jump::Register(r) => {
                        let target = self.registers.er[usize::from(r)] as u16;
                        return self.jump_target(bus, target, return_pc);
                    }
                    Jump::Vector(address) => {
                        let target = self.read(bus, u16::from(address) & !1, Width::Word, false)?;
                        if let Some(return_pc) = return_pc {
                            self.push_return(bus, return_pc)?;
                            return self.jump_target(bus, target, None);
                        }
                        self.idle(bus, 2)?;
                        return self.jump_target(bus, target, None);
                    }
                }
            }
            Instruction::Return { exception } => {
                let mut target = self.pop(bus)?;
                if exception {
                    let ccr = (target >> 8) as u8;
                    target = self.pop(bus)?;
                    // RTE is not one of §3.8.5's CCR-writing deferral instructions.
                    self.registers.ccr = ccr;
                }
                self.idle(bus, 2)?;
                return self.jump_target(bus, target, None);
            }
            Instruction::Trap(vector) => {
                self.retire()?;
                self.enter_exception(8 + vector, Entry::Trap);
                return Ok(());
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
            }
            Instruction::StoreCcr(r) => {
                self.registers
                    .write(Size::Byte, r, u32::from(self.registers.ccr));
            }
            Instruction::MulDiv {
                divide,
                signed,
                size,
                src,
                dst,
            } => {
                let (out_size, value, flags) =
                    self.registers.muldiv(divide, signed, size, src, dst);
                // The prefix and NEXT fetches are physical bus actions. The table
                // assigns a further 12/20 internal states for byte/word operands.
                self.idle(bus, if size == Size::Byte { 12 } else { 20 })?;
                self.write_register(out_size, dst, value);
                self.registers.ccr = flags;
            }
            Instruction::Decimal { subtract, reg } => self.decimal(subtract, reg),
            Instruction::EepMov { word_count } => {
                self.read(bus, self.registers.er[5] as u16, Width::Byte, false)?;
                self.read(bus, self.registers.er[6] as u16, Width::Byte, false)?;
                if Self::copy_count(&self.registers, word_count) != 0 {
                    self.end_work(State::Boundary)?;
                    self.begin_work(Work::Copy { word_count });
                    return Ok(());
                }
                self.fetch_next(bus, true)?;
            }
        }
        self.retire()
    }
    fn copy_count(registers: &Registers, word_count: bool) -> u32 {
        if word_count {
            registers.read(Size::Word, 4)
        } else {
            registers.read(Size::Byte, 12)
        }
    }
    fn copy_pair(&mut self, bus: &mut impl Bus, word_count: bool) -> Result<(), Stop> {
        if word_count && self.nmi_between_pairs(bus)? {
            // The NEXT instruction is saved, and R4/ER5/ER6 describe the
            // remaining copy. .B defers even NMI; a pair is never split.
            self.retire()?;
            self.enter_exception(7, Entry::Fetch);
            return Ok(());
        }
        let value = self.read(bus, self.registers.er[5] as u16, Width::Byte, false)?;
        self.write(
            bus,
            self.registers.er[6] as u16,
            Width::Byte,
            value,
            WriteOrigin::Other,
        )?;
        for index in [5, 6] {
            let r = self.er_mut(index);
            *r = r.wrapping_add(1);
        }
        let count = Self::copy_count(&self.registers, word_count);
        if word_count {
            self.write_register(Size::Word, 4, count.wrapping_sub(1));
        } else {
            self.write_register(Size::Byte, 12, count.wrapping_sub(1));
        }
        if count == 1 {
            self.fetch_next(bus, true)?;
            return self.retire();
        }
        self.end_work(State::Boundary)?;
        self.begin_work(Work::Copy { word_count });
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn transfer(
        &mut self,
        bus: &mut impl Bus,
        size: Size,
        reg: u8,
        address: Address,
        store: bool,
        ccr: bool,
        short: bool,
    ) -> Result<(), Stop> {
        // MOV @-ERn updates the full address register before reading an
        // aliased source (RnH/RnL/Rn/En/ERn). ADE-602-053A MOV.B/W/L usage
        // notes, pp. 121/123/125. Loads still commit a post-increment before
        // replacing the destination field.
        let updating = matches!(
            address,
            Address::PreDecrement(_) | Address::PostIncrement(_)
        );
        let absolute8 = matches!(address, Address::Absolute(_)) && short;
        let (mut address, post) = self.target_address(address, size);
        let value = if ccr {
            u32::from(self.registers.ccr) << 8
        } else {
            self.registers.read(size, reg)
        };
        if size != Size::Byte {
            address &= !1;
        }
        if updating {
            self.idle(bus, 2)?;
        }
        let width = if size == Size::Byte {
            Width::Byte
        } else {
            Width::Word
        };
        let origin = match (width, ccr, absolute8) {
            (Width::Byte, false, true) => WriteOrigin::MovByteAbs8 {
                pc_bit1: self.instruction_pc & 2 != 0,
            },
            (Width::Byte, false, false) => WriteOrigin::MovByte,
            _ => WriteOrigin::Other,
        };
        let mut loaded = 0u32;
        let mut done = 0u16;
        while done < size.bytes() {
            let lane = address.wrapping_add(done);
            if store {
                let shift = (size.bytes() - done - u16::from(width.bytes())) * 8;
                self.write(bus, lane, width, (value >> shift) as u16, origin)?;
            } else {
                loaded = (loaded << 16) | u32::from(self.read(bus, lane, width, false)?);
            }
            done += u16::from(width.bytes());
        }
        if let Some((r, v)) = post {
            *self.er_mut(r) = v;
        }
        if ccr {
            if !store {
                self.registers.ccr = (loaded >> 8) as u8;
                self.interrupt_delay = 1;
            }
        } else {
            let value = if store {
                value
            } else {
                self.write_register(size, reg, loaded);
                loaded
            };
            self.registers.ccr = alu::logical(value, size, self.registers.ccr);
        }
        Ok(())
    }
    fn jump_target(
        &mut self,
        bus: &mut impl Bus,
        target: u16,
        return_pc: Option<u16>,
    ) -> Result<(), Stop> {
        let word = self.read(bus, target & !1, Width::Word, true)?;
        self.registers.pc = target & !1;
        self.prefetch = Prefetch::new(target & !1, word);
        if let Some(return_pc) = return_pc {
            self.push_return(bus, return_pc)?;
        }
        self.retire()
    }
    fn push_return(&mut self, bus: &mut impl Bus, return_pc: u16) -> Result<(), Stop> {
        self.move_sp(-2);
        self.write(
            bus,
            self.registers.sp() & !1,
            Width::Word,
            return_pc,
            WriteOrigin::Other,
        )
    }
    fn pop(&mut self, bus: &mut impl Bus) -> Result<u16, Stop> {
        let value = self.read(bus, self.registers.sp() & !1, Width::Word, false)?;
        self.move_sp(2);
        Ok(value)
    }

    // Register-only semantics.

    fn source(&self, size: Size, source: Source) -> u32 {
        match source {
            Source::Reg(r) => self.registers.read(size, r),
            Source::Imm(v) => v & size.mask(),
        }
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
                let er = self.er_mut(r);
                *er = er.wrapping_sub(u32::from(size.bytes()));
                (*er as u16, None)
            }
        }
    }
    fn decimal(&mut self, subtract: bool, reg: u8) {
        let (r, ccr) = decimal(
            self.registers.read(Size::Byte, reg) as u8,
            subtract,
            self.registers.ccr,
        );
        self.write_register(Size::Byte, reg, u32::from(r));
        self.registers.ccr = ccr;
    }
}
impl Registers {
    fn muldiv(&self, divide: bool, signed: bool, size: Size, src: u8, dst: u8) -> (Size, u32, u8) {
        let out_size = if size == Size::Byte {
            Size::Word
        } else {
            Size::Long
        };
        let divisor = self.read(size, src);
        let dividend = self.read(out_size, dst);
        let mut flags = self.ccr;
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
        (out_size, value, flags)
    }
}

/// DAA/DAS on `a` with the flags in `ccr`: the adjusted value and new CCR.
fn decimal(a: u8, subtract: bool, ccr: u8) -> (u8, u8) {
    let h = ccr & H != 0;
    let c = ccr & C != 0;
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
    // The model preserves H and V, whose results are unspecified.
    let mut ccr = (ccr & !(N | Z)) | alu::nz(u32::from(r), Size::Byte);
    if !subtract && (u16::from(a) + u16::from(correction) > 255) {
        ccr |= C;
    }
    (r, ccr)
}
