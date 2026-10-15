//! Direct execution of memory-only work.
//!
//! An interval fixes the clock configuration and external causes. Ordinary
//! memory accesses need only CPU-edge accounting; owner-local register accesses
//! project their completion time when the owner needs it. Other accesses return
//! to the timed bus before changing anything.
//!
//! Cached flash instructions form bounded register-only prefixes followed by
//! at most one memory, admission or control instruction. The whole prefix must
//! fit strictly inside the window before it runs. Pending interrupt deferral
//! and short horizons select a single instruction. RAM validates fetched words
//! and protects retained prefetch on every instruction.
//!
//! Every effect follows the timed sequences of `mod.rs`: the same accesses in
//! the same order, the same arithmetic in `alu`, the same retained prefetch.
//! Only the bookkeeping that suspension needs is absent, because declined work
//! has not begun. Each cached instruction carries a handler specialized for
//! its operation and operand widths, so executing it takes one dispatch.
//! The cache is not state: entries are validated against the code they were
//! decoded from, and copies start empty.
use super::alu::{self, I};
use super::decode::{
    self, Address, Alu, Bit, BitIndex, CcrOp, Decode, Instruction, Size, Source, Target,
};
use super::execution::Mark;
use super::{Cpu, Prefetch, Registers, State, Width, Work, WriteOrigin};
use crate::mcu::{FLASH_SIZE, RAM_SIZE, RAM_START};

/// Memory that memory-only work reads and writes without the board.
pub(crate) struct Memory<'a> {
    pub ram: &'a mut [u8; RAM_SIZE],
    pub flash: &'a [u8; FLASH_SIZE],
    /// Changes whenever flash cells change.
    pub generation: u64,
    /// Register accesses that complete without the board.
    pub registers: &'a mut dyn RegisterBus,
}

/// Register accesses whose consequences stay with their owners. An access
/// the bus cannot complete that way is declined with nothing changed.
pub(crate) trait RegisterBus {
    /// Access `address` with `width`, starting `begin` CPU clock edges into
    /// the window.
    fn access(
        &mut self,
        address: u16,
        width: Width,
        write: Option<(u16, WriteOrigin)>,
        begin: u64,
    ) -> Option<Register>;
}
/// A completed register access.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Register {
    pub value: u16,
    pub states: u8,
    /// The access changed state, or observed state that can change (`Mark`).
    pub disturbs: bool,
    /// The offered request may have changed: the run ends after this
    /// instruction so its bus can derive it again.
    pub stop: bool,
    /// For an access that does not disturb: the last CPU clock position
    /// at which the same access completes the same way, if earlier than the
    /// window's end.
    pub until: u64,
}

/// The window and counters of an interval, as memory-only work sees them.
pub(crate) struct DirectRun {
    /// CPU clock edges spent in the window, and edges that complete strictly
    /// inside it.
    pub elapsed: u64,
    pub available: u64,
    /// The CPU clock ordinal at the window's start.
    pub base: u64,
    /// The offered request, constant while only memory is accessed.
    pub interrupt: Option<u8>,
    /// Disturbance generation (see `Mark`), and completed physical accesses.
    pub disturbances: u64,
    pub reads: u64,
    pub writes: u64,
    /// The earliest `Register::until` since the last disturbance: no
    /// repetition of work since then completes an access after it.
    pub horizon: u64,
}
impl DirectRun {
    fn mark(&self) -> Mark {
        Mark {
            position: self.base + self.elapsed,
            disturbances: self.disturbances,
            reads: self.reads,
            writes: self.writes,
        }
    }
}

const FLASH_ENTRIES: usize = FLASH_SIZE / 2;
const RAM_ENTRIES: usize = RAM_SIZE / 2;
/// The largest extra states a register access adds to a memory access.
const REGISTER_EXTRA: u8 = 1;
/// The most states a register access may take to complete here; the
/// register bus declines slower ones.
pub(crate) const REGISTER_STATES: u8 = 2 + REGISTER_EXTRA;

type BusHandler = fn(&mut Registers, &mut Lanes<'_, '_>, &Entry) -> Flow;
type PureHandler = fn(&mut Registers, &Entry);
#[derive(Clone, Copy)]
enum Handler {
    /// No memory, PC dependence or change to CCR.I or interrupt admission.
    Pure(PureHandler),
    /// May access memory, use PC or defer interrupt admission.
    Bus(BusHandler),
    /// Qualified target and condition derived from the canonical branch rule.
    Branch,
    ReadByteAbsolute,
    WriteByteAbsolute,
}

/// A register-only prefix and, when present, its first consequential instruction.
#[derive(Clone, Copy)]
struct Run {
    pure: u8,
    tail: bool,
    cost: u16,
    states: u16,
    reads: u8,
    writes: u8,
}
impl Run {
    const EMPTY: Self = Self {
        pure: 0,
        tail: false,
        cost: 0,
        states: 0,
        reads: 0,
        writes: 0,
    };
    fn push(&mut self, entry: &Entry) {
        self.cost += u16::from(entry.cost);
        match entry.handler {
            Handler::Pure(_) => {
                self.pure += 1;
                self.states += u16::from(entry.states);
                self.reads += entry.accesses as u8;
                self.writes += (entry.accesses >> 32) as u8;
            }
            _ => self.tail = true,
        }
    }
    fn single(entry: &Entry) -> Self {
        let mut run = Self::EMPTY;
        run.push(entry);
        run
    }
}

#[derive(Clone, Copy)]
struct Entry {
    handler: Handler,
    run: Run,
    origin_pc_bit1: bool,
    /// A branch target word in the same immutable flash generation.
    target_word: Option<u16>,
    /// Operand fields: the destination or transferred register, and the
    /// source, address or bit register.
    x: u8,
    y: u8,
    len: u8,
    /// States of the whole instruction when every access is to memory, and
    /// (`cost`) at most when its data access reaches a register instead.
    states: u8,
    cost: u8,
    /// Reads in the low half and writes in the high half.
    accesses: u64,
    /// Immediate, absolute address, displacement or offset. Bcc stores its
    /// condition truth table in the high half and target in the low half.
    imm: u32,
    /// Flash generation, or zero for an empty entry.
    generation: u64,
    /// The decoded words, which RAM code must still hold.
    words: [u16; 5],
}
fn empty(_: &mut Registers, _: &mut Lanes<'_, '_>, _: &Entry) -> Flow {
    Flow::Decline
}
const EMPTY: Entry = Entry {
    handler: Handler::Bus(empty),
    run: Run::EMPTY,
    origin_pc_bit1: false,
    target_word: None,
    x: 0,
    y: 0,
    len: 0,
    states: 0,
    cost: 0,
    accesses: 0,
    imm: 0,
    generation: 0,
    words: [0; 5],
};

/// Decoded instructions by word address. A CPU made for running owns them
/// from the start, so ordinary runs never allocate. Non-running CPU projections
/// and snapshots leave these disposable tables empty.
pub(crate) struct Code(Option<Tables>);
struct Tables {
    flash: Box<[Entry; FLASH_ENTRIES]>,
    ram: Box<[Entry; RAM_ENTRIES]>,
}
impl Tables {
    fn new() -> Self {
        Self {
            flash: table(),
            ram: table(),
        }
    }
}
fn table<const N: usize>() -> Box<[Entry; N]> {
    vec![EMPTY; N]
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!())
}
impl Code {
    pub fn empty() -> Self {
        Self(None)
    }
    /// Own the tables now rather than on first use.
    pub fn prepare(&mut self) {
        self.0.get_or_insert_with(Tables::new);
    }
}
impl Clone for Code {
    fn clone(&self) -> Self {
        Self(None)
    }
}
impl PartialEq for Code {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Code {}
impl core::fmt::Debug for Code {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Code")
    }
}
/// The entry for RAM code at `pc`, decoded again unless memory still holds
/// the words it was decoded from.
fn ram_entry<'c>(
    memory: &mut Memory<'_>,
    entries: &'c mut [Entry; RAM_ENTRIES],
    pc: u16,
) -> Option<&'c Entry> {
    if !(RAM_START..0xff7f).contains(&pc) {
        return None;
    }
    let r = usize::from(pc - RAM_START);
    let entry = &mut entries[r >> 1];
    let ram = &*memory.ram;
    let valid = entry.generation != 0
        && (0..usize::from(entry.len)).all(|k| {
            let j = r + 2 * k;
            j + 1 < RAM_SIZE && u16::from_be_bytes([ram[j], ram[j + 1]]) == entry.words[k]
        });
    if !valid {
        *entry = decode_at(memory, pc, 1)?;
    }
    Some(entry)
}

/// Prepare at most sixteen entries without allocating. The first entry's
/// generation qualifies the whole run: flash cannot change inside an interval.
/// RAM execution keeps per-instruction word validation and prefetch guards.
#[inline(never)]
fn prepare_run(
    memory: &mut Memory<'_>,
    entries: &mut [Entry; FLASH_ENTRIES],
    pc: u16,
    generation: u64,
) -> Option<Run> {
    let mut cursor = pc;
    let mut run = Run::EMPTY;
    for _ in 0..16 {
        if usize::from(cursor) >= FLASH_SIZE {
            break;
        }
        let slot = &mut entries[usize::from(cursor >> 1)];
        if slot.generation != generation {
            let Some(decoded) = decode_at(memory, cursor, generation) else {
                break;
            };
            *slot = decoded;
        }
        run.push(slot);
        if run.tail {
            break;
        }
        cursor = cursor.wrapping_add(2 * u16::from(slot.len));
    }
    if run.pure == 0 && !run.tail {
        return None;
    }
    entries[usize::from(pc >> 1)].run = run;
    Some(run)
}

/// Count the canonical instruction sequence on ordinary memory. Values and
/// branch direction do not change the access count of the admitted families.
fn timing(instruction: &Instruction, len: usize) -> Option<(u8, u8, u8, u8)> {
    if matches!(
        instruction,
        Instruction::Sleep | Instruction::Trap(_) | Instruction::EepMov { .. }
    ) {
        return None;
    }
    struct Count {
        states: u8,
        reads: u8,
        writes: u8,
    }
    impl super::execution::Bus for Count {
        fn read(&mut self, _: u16, _: Width, _: bool) -> Result<u16, super::execution::Stop> {
            self.states += 2;
            self.reads += 1;
            Ok(0)
        }
        fn write(
            &mut self,
            _: u16,
            _: Width,
            _: u16,
            _: WriteOrigin,
        ) -> Result<(), super::execution::Stop> {
            self.states += 2;
            self.writes += 1;
            Ok(())
        }
        fn idle(&mut self, states: u32) -> Result<(), super::execution::Stop> {
            self.states += states as u8;
            Ok(())
        }
        fn sample(&mut self) -> Result<Option<u8>, super::execution::Stop> {
            Ok(None)
        }
    }
    // Extension words were fetched before `decoded`; the first word was
    // retained by the preceding instruction and is not charged twice.
    let ext = (len - 1) as u8;
    let mut count = Count {
        states: 2 * ext,
        reads: ext,
        writes: 0,
    };
    let mut cpu = Cpu::bare();
    cpu.registers.pc = (2 * len) as u16;
    cpu.begin_work(Work::Instruction);
    cpu.decoded(
        &mut count,
        *instruction,
        super::memo::Prelude::of(instruction, len),
        len,
    )
    .ok()?;
    // Only these data accesses may reach owner-local registers in this
    // executor. Multi-lane transfers and stack operations require memory.
    let slack = u8::from(matches!(
        instruction,
        Instruction::Memory {
            size: Size::Byte | Size::Word,
            ..
        } | Instruction::Bit {
            target: Target::Memory(_),
            op: Bit::Test | Bit::Load(_) | Bit::And(_) | Bit::Or(_) | Bit::Xor(_),
            ..
        }
    ));
    Some((count.states, slack, count.reads, count.writes))
}

fn bit_writes(op: Bit) -> bool {
    matches!(op, Bit::Set | Bit::Clear | Bit::Not | Bit::Store(_))
}

/// Memory reads and writes of one instruction. A declined access leaves
/// everything as it was before the instruction began.
struct Lanes<'b, 'm> {
    memory: &'b mut Memory<'m>,
    /// Window edges spent before the current instruction, beyond `extra`.
    elapsed: u64,
    /// A register access asked the run to end after this instruction.
    stop: bool,
    /// States beyond two for accesses that reached registers.
    extra: u32,
    /// An access disturbed the board since the last published branch mark.
    disturbed: bool,
    /// `DirectRun::horizon`.
    horizon: u64,
    /// The word holding the next instruction's first word when code runs
    /// from RAM: a write there would change what a later fetch reads while
    /// the prefetch keeps the old word, so the interval bus performs it.
    guard: u16,
}
impl Lanes<'_, '_> {
    #[inline(always)]
    fn read(&mut self, address: u16, width: Width) -> Option<u16> {
        let a = if width == Width::Word {
            address & !1
        } else {
            address
        };
        let i = usize::from(a);
        if i < FLASH_SIZE {
            let flash = self.memory.flash;
            return Some(match width {
                Width::Word => u16::from_be_bytes([flash[i], flash[i + 1]]),
                Width::Byte => u16::from(flash[i]),
            });
        }
        if (RAM_START..=0xff7f).contains(&a) {
            let i = usize::from(a - RAM_START);
            let ram = &*self.memory.ram;
            return Some(match width {
                Width::Word => u16::from_be_bytes([ram[i], ram[i + 1]]),
                Width::Byte => u16::from(ram[i]),
            });
        }
        None
    }
    /// A data access `offset` states into the instruction: memory as `read`
    /// and `write`, or a register through the register bus.
    #[inline(always)]
    fn data_read(&mut self, address: u16, width: Width, offset: u8) -> Option<u16> {
        match self.read(address, width) {
            Some(v) => Some(v),
            None => self.register(address, width, None, offset),
        }
    }
    #[inline(never)]
    fn register(
        &mut self,
        address: u16,
        width: Width,
        write: Option<(u16, WriteOrigin)>,
        offset: u8,
    ) -> Option<u16> {
        let a = if width == Width::Word {
            address & !1
        } else {
            address
        };
        if (RAM_START..=0xff7f).contains(&a) || usize::from(a) < FLASH_SIZE {
            return None;
        }
        let begin = self.elapsed + u64::from(self.extra) + u64::from(offset);
        let done = self.memory.registers.access(a, width, write, begin)?;
        // `cost` admitted the instruction for this (see `REGISTER_STATES`).
        debug_assert!((2..=REGISTER_STATES).contains(&done.states));
        self.extra += u32::from(done.states - 2);
        self.disturbed |= done.disturbs;
        self.horizon = if done.disturbs {
            u64::MAX
        } else {
            self.horizon.min(done.until)
        };
        self.stop |= done.stop;
        Some(done.value)
    }
    /// A data write `offset` states into the instruction, or `false` with
    /// nothing changed.
    #[inline(always)]
    fn data_write(
        &mut self,
        address: u16,
        width: Width,
        value: u16,
        origin: WriteOrigin,
        offset: u8,
    ) -> bool {
        if self.writable(address, width) {
            self.write(address, width, value);
            return true;
        }
        self.register(address, width, Some((value, origin)), offset)
            .is_some()
    }
    /// Whether a write of `width` at `address` can complete here.
    #[inline(always)]
    fn writable(&self, address: u16, width: Width) -> bool {
        let a = if width == Width::Word {
            address & !1
        } else {
            address
        };
        (RAM_START..=0xff7f).contains(&a) && a & !1 != self.guard
    }
    /// A write already found `writable`.
    #[inline(always)]
    fn write(&mut self, address: u16, width: Width, value: u16) {
        let a = if width == Width::Word {
            address & !1
        } else {
            address
        };
        let i = usize::from(a - RAM_START);
        let ram = &mut *self.memory.ram;
        let changed = match width {
            Width::Word => {
                let bytes = value.to_be_bytes();
                let changed = ram[i..i + 2] != bytes;
                ram[i..i + 2].copy_from_slice(&bytes);
                changed
            }
            Width::Byte => {
                let changed = ram[i] != value as u8;
                ram[i] = value as u8;
                changed
            }
        };
        self.disturbed |= changed;
    }
}

#[derive(Clone, Copy)]
enum Flow {
    /// The instruction completed.
    Next,
    /// A taken backward branch completed.
    Backward,
    /// The instruction completed and defers interrupt acceptance past the
    /// next boundary (§3.8.5).
    Defer,
    /// The instruction was declined before it changed anything.
    Decline,
}

impl Cpu {
    /// Run memory-only work until an instruction is declined.
    pub(crate) fn run_direct(&mut self, memory: &mut Memory<'_>, run: &mut DirectRun) {
        let retired = self.retired;
        let mut code = self.code.0.take().unwrap_or_else(Tables::new);
        self.direct_loop(memory, run, &mut code);
        self.code.0 = Some(code);
        if self.retired != retired {
            // NEXT of the last instruction, which no later write changed.
            let pc = self.registers.pc;
            self.prefetch = code_word(memory, pc).map_or(Prefetch::NONE, |w| Prefetch::new(pc, w));
        }
    }
    #[inline(always)]
    fn direct_loop(&mut self, memory: &mut Memory<'_>, run: &mut DirectRun, code: &mut Tables) {
        // Admission already began an instruction whose work has not started.
        let begun = match self.state {
            State::Boundary => false,
            State::Busy(Work::Instruction)
                if self.accepted_vector.is_none() && self.completed.count == 0 =>
            {
                true
            }
            _ => return,
        };
        if !self.direct_entry(memory) {
            return;
        }
        let mut begun = begun;
        loop {
            let flash = usize::from(self.registers.pc) < FLASH_SIZE;
            let retired = self.retired;
            match (run.interrupt.is_some(), flash) {
                (false, true) => self.direct_instructions::<false, true>(memory, run, code, begun),
                (false, false) => {
                    self.direct_instructions::<false, false>(memory, run, code, begun)
                }
                (true, true) => self.direct_instructions::<true, true>(memory, run, code, begun),
                (true, false) => self.direct_instructions::<true, false>(memory, run, code, begun),
            }
            // A control transfer between flash and RAM changes the cache's
            // qualification, not the instruction semantics or clock window.
            if self.retired == retired || (usize::from(self.registers.pc) < FLASH_SIZE) == flash {
                break;
            }
            begun = false;
        }
    }

    /// Execute instructions until one is declined or, when `OFFERED`, the
    /// offered request becomes admissible at a boundary.
    #[inline(never)]
    fn direct_instructions<const OFFERED: bool, const FLASH: bool>(
        &mut self,
        memory: &mut Memory<'_>,
        run: &mut DirectRun,
        code: &mut Tables,
        begun: bool,
    ) {
        let generation = memory.generation;
        let nmi = run.interrupt == Some(7);
        // Bound unflushed physical-access totals to their packed 32-bit lanes.
        // Recognized repetitions publish their counts directly as u64 totals.
        let available = run
            .available
            .min(run.elapsed.saturating_add(u64::from(u32::MAX)));
        let mut elapsed = run.elapsed;
        // Reads in the low half, writes in the high half.
        let mut accesses = 0u64;
        let initial_retired = self.retired;
        let mut retired = initial_retired;
        // A begun instruction was already admitted, including its deferral.
        let mut delay = self.interrupt_delay + u8::from(begun);
        let mut instruction_pc = self.instruction_pc;
        let mut lanes = Lanes {
            memory,
            elapsed: 0,
            stop: false,
            extra: 0,
            disturbed: false,
            horizon: run.horizon,
            guard: 0,
        };
        'work: loop {
            let pc = self.registers.pc;
            if OFFERED && delay == 0 && (nmi || self.registers.ccr & I == 0) {
                break;
            }
            if (usize::from(pc) < FLASH_SIZE) != FLASH {
                break;
            }
            let (mut selected, base) = if FLASH {
                let first = &code.flash[usize::from(pc >> 1)];
                let selected =
                    if first.generation == generation && (first.run.pure != 0 || first.run.tail) {
                        first.run
                    } else {
                        match prepare_run(lanes.memory, &mut code.flash, pc, generation) {
                            Some(run) => run,
                            None => break,
                        }
                    };
                lanes.guard = 0;
                (selected, 0)
            } else {
                let Some(entry) = ram_entry(lanes.memory, &mut code.ram, pc) else {
                    break;
                };
                lanes.guard = pc.wrapping_add(2 * u16::from(entry.len));
                (Run::single(entry), RAM_START)
            };
            let entries: &[Entry] = if FLASH {
                &code.flash[..]
            } else {
                &code.ram[..]
            };
            let begin = elapsed + u64::from(lanes.extra);
            if delay != 0 || begin + u64::from(selected.cost) > available {
                let first = &entries[usize::from(pc - base) >> 1];
                selected = Run::single(first);
            }
            if begin + u64::from(selected.cost) > available {
                break;
            }
            let mut cursor = pc;
            if selected.pure != 0 {
                let (next, last) = pure_prefix(
                    &mut self.registers,
                    entries,
                    usize::from(pc.wrapping_sub(base)) >> 1,
                    selected.pure,
                );
                cursor = base.wrapping_add(2 * next as u16);
                instruction_pc = base.wrapping_add(2 * last as u16);
            }
            elapsed += u64::from(selected.states);
            accesses += u64::from(selected.reads) | u64::from(selected.writes) << 32;
            retired = retired.wrapping_add(u64::from(selected.pure));
            delay = delay.saturating_sub(selected.pure);
            if selected.tail {
                let pc = cursor;
                let entry = &entries[usize::from(pc.wrapping_sub(base)) >> 1];
                let extra = lanes.extra;
                cursor = pc.wrapping_add(2 * u16::from(entry.len));
                self.registers.pc = cursor;
                lanes.elapsed = elapsed;
                let flow = match entry.handler {
                    Handler::Branch => {
                        // The truth table is derived from the shared condition
                        // rule at decode, indexed by CCR's C/V/Z/N fields.
                        let taken = (entry.imm >> 16) >> (self.registers.ccr & 15) & 1 != 0;
                        if taken {
                            self.registers.pc = entry.imm as u16;
                        }
                        if taken && entry.y != 0 {
                            Flow::Backward
                        } else {
                            Flow::Next
                        }
                    }
                    Handler::ReadByteAbsolute => {
                        transfer::<0, false, 0, false>(&mut self.registers, &mut lanes, entry)
                    }
                    Handler::WriteByteAbsolute => {
                        transfer::<0, true, 0, false>(&mut self.registers, &mut lanes, entry)
                    }
                    Handler::Bus(handler) => handler(&mut self.registers, &mut lanes, entry),
                    Handler::Pure(_) => unreachable!("prepared terminal instruction"),
                };
                match flow {
                    Flow::Next => {}
                    Flow::Decline => {
                        self.registers.pc = pc;
                        lanes.extra = extra;
                        break 'work;
                    }
                    Flow::Defer => delay = 2,
                    Flow::Backward => {}
                }
                elapsed = elapsed.wrapping_add(u64::from(entry.states));
                accesses = accesses.wrapping_add(entry.accesses);
                retired = retired.wrapping_add(1);
                delay = delay.saturating_sub(1);
                instruction_pc = pc;
                if lanes.stop {
                    break 'work;
                }
                if let Flow::Backward = flow {
                    run.elapsed = elapsed + u64::from(lanes.extra);
                    run.disturbances += u64::from(lanes.disturbed);
                    run.horizon = lanes.horizon;
                    run.reads = run.reads.wrapping_add(accesses & 0xffff_ffff);
                    run.writes = run.writes.wrapping_add(accesses >> 32);
                    self.retired = retired;
                    (lanes.extra, lanes.disturbed, accesses) = (0, false, 0);
                    self.direct_recognize(lanes.memory, run, delay, entry.target_word);
                    retired = self.retired;
                    elapsed = run.elapsed;
                }
                cursor = self.registers.pc;
            }
            self.registers.pc = cursor;
        }

        run.elapsed = elapsed + u64::from(lanes.extra);
        run.disturbances += u64::from(lanes.disturbed);
        run.horizon = lanes.horizon;
        run.reads = run.reads.wrapping_add(accesses & 0xffff_ffff);
        run.writes = run.writes.wrapping_add(accesses >> 32);
        self.retired = retired;
        if retired != initial_retired {
            self.instruction_pc = instruction_pc;
            self.interrupt_delay = delay;
            if begun {
                self.state = State::Boundary;
            }
        }
    }
    /// The retained prefetch must be the word memory holds at the PC, as it
    /// stays while only this executor runs.
    fn direct_entry(&mut self, memory: &mut Memory<'_>) -> bool {
        let pc = self.registers.pc;
        let Some((address, word)) = self.prefetch.get() else {
            return false;
        };
        address == pc && code_word(memory, pc) == Some(word)
    }
    /// A taken backward branch retired: account for repetitions of the
    /// iteration it closed when that iteration provably repeats.
    #[inline(always)]
    fn direct_recognize(
        &mut self,
        memory: &mut Memory<'_>,
        run: &mut DirectRun,
        delay: u8,
        target_word: Option<u16>,
    ) {
        let mark = run.mark();
        let pc = self.registers.pc;
        let prefetch = target_word
            .or_else(|| code_word(memory, pc))
            .map_or(Prefetch::NONE, |w| Prefetch::new(pc, w));
        let Some((since, retired)) =
            self.repetition
                .arrive(&self.registers, prefetch, delay, mark, self.retired)
        else {
            return;
        };
        let period = mark.position.wrapping_sub(since.position);
        let limit = run.available.min(run.horizon.saturating_sub(run.base));
        let remaining = limit.saturating_sub(run.elapsed);
        let repetitions = mark.repetitions_since(&since, remaining);
        if repetitions == 0 {
            return;
        }
        run.elapsed += repetitions * period;
        let (reads, writes) = mark.accesses_since(&since, repetitions);
        run.reads = run.reads.wrapping_add(reads);
        run.writes = run.writes.wrapping_add(writes);
        self.retired = self.retired.wrapping_add(repetitions.wrapping_mul(retired));
        self.repetition.advanced(run.mark(), self.retired);
    }
}

/// Run a qualified register-only prefix. Keeping only its cursor and register
/// file live here avoids carrying bus accounting across every handler call.
#[inline(never)]
fn pure_prefix(
    registers: &mut Registers,
    entries: &[Entry],
    mut index: usize,
    count: u8,
) -> (usize, usize) {
    let mut last = index;
    for _ in 0..count {
        let entry = &entries[index];
        let Handler::Pure(handler) = entry.handler else {
            unreachable!("prepared register-only prefix");
        };
        handler(registers, entry);
        last = index;
        index += usize::from(entry.len);
    }
    (index, last)
}

// Handlers. Each performs one instruction's effects as `execute_instruction`
// does over memory, with its operation and widths fixed by const parameters.
// Bus handlers receive the fall-through PC; pure handlers cannot observe PC.

const fn size_of(n: u8) -> Size {
    match n {
        0 => Size::Byte,
        1 => Size::Word,
        _ => Size::Long,
    }
}
const fn size_code(size: Size) -> u8 {
    size as u8
}
const fn alu_of(n: u8) -> Alu {
    match n {
        0 => Alu::Mov,
        1 => Alu::Add,
        2 => Alu::Addx,
        3 => Alu::Sub,
        4 => Alu::Subx,
        5 => Alu::Cmp,
        6 => Alu::And,
        7 => Alu::Or,
        _ => Alu::Xor,
    }
}
const fn alu_code(op: Alu) -> u8 {
    match op {
        Alu::Mov => 0,
        Alu::Add => 1,
        Alu::Addx => 2,
        Alu::Sub => 3,
        Alu::Subx => 4,
        Alu::Cmp => 5,
        Alu::And => 6,
        Alu::Or => 7,
        Alu::Xor => 8,
    }
}

fn binary<const OP: u8, const SIZE: u8, const IMM: bool>(cpu: &mut Registers, e: &Entry) {
    let (op, size) = (alu_of(OP), size_of(SIZE));
    let r = &mut *cpu;
    let a = r.read(size, e.x);
    let b = if IMM {
        e.imm & size.mask()
    } else {
        r.read(size, e.y)
    };
    let (v, f) = alu::binary(op, size, a, b, r.ccr);
    if op != Alu::Cmp {
        r.write(size, e.x, v);
    }
    r.ccr = f;
}
fn unary<const OP: u8, const SIZE: u8>(cpu: &mut Registers, e: &Entry) {
    let op = match OP {
        0 => decode::Unary::Not,
        1 => decode::Unary::Neg,
        2 => decode::Unary::Inc,
        3 => decode::Unary::Dec,
        4 => decode::Unary::Extu,
        _ => decode::Unary::Exts,
    };
    let size = size_of(SIZE);
    let r = &mut *cpu;
    let (v, f) = alu::unary(op, size, r.read(size, e.x), e.imm, r.ccr);
    r.write(size, e.x, v);
    r.ccr = f;
}
fn shift<const OP: u8, const SIZE: u8>(cpu: &mut Registers, e: &Entry) {
    let op = match OP {
        0 => decode::Shift::Shll,
        1 => decode::Shift::Shal,
        2 => decode::Shift::Shlr,
        3 => decode::Shift::Shar,
        4 => decode::Shift::Rotl,
        5 => decode::Shift::Rotr,
        6 => decode::Shift::Rotxl,
        _ => decode::Shift::Rotxr,
    };
    let size = size_of(SIZE);
    let r = &mut *cpu;
    let (v, f) = alu::shift(op, size, r.read(size, e.x), r.ccr);
    r.write(size, e.x, v);
    r.ccr = f;
}
fn quick(cpu: &mut Registers, e: &Entry) {
    let er = &mut cpu.er[usize::from(e.x & 7)];
    *er = er.wrapping_add(e.imm);
}
fn nop(_: &mut Registers, _: &Entry) {}
/// The bit operation `e.imm` names (see `bit_code`).
fn bit_op(code: u32) -> Bit {
    let inv = code & 16 != 0;
    match code & 15 {
        0 => Bit::Set,
        1 => Bit::Clear,
        2 => Bit::Not,
        3 => Bit::Test,
        4 => Bit::Store(inv),
        5 => Bit::Load(inv),
        6 => Bit::And(inv),
        7 => Bit::Or(inv),
        _ => Bit::Xor(inv),
    }
}
fn bit_code(op: Bit) -> u32 {
    match op {
        Bit::Set => 0,
        Bit::Clear => 1,
        Bit::Not => 2,
        Bit::Test => 3,
        Bit::Store(inv) => 4 | u32::from(inv) << 4,
        Bit::Load(inv) => 5 | u32::from(inv) << 4,
        Bit::And(inv) => 6 | u32::from(inv) << 4,
        Bit::Or(inv) => 7 | u32::from(inv) << 4,
        Bit::Xor(inv) => 8 | u32::from(inv) << 4,
    }
}
/// Fix the bit operation once, while retaining the shared arithmetic rule.
fn bit_handler(op: u32, register_bit: bool, mode: u8) -> Handler {
    fn form<const OP: u32>(register_bit: bool, mode: u8) -> Handler {
        match (mode, register_bit) {
            (0, false) => Handler::Pure(bit_register::<false, OP>),
            (0, true) => Handler::Pure(bit_register::<true, OP>),
            (1, false) => Handler::Bus(bit_memory::<false, false, OP>),
            (1, true) => Handler::Bus(bit_memory::<true, false, OP>),
            (_, false) => Handler::Bus(bit_memory::<false, true, OP>),
            (_, true) => Handler::Bus(bit_memory::<true, true, OP>),
        }
    }
    match op {
        0 => form::<0>(register_bit, mode),
        1 => form::<1>(register_bit, mode),
        2 => form::<2>(register_bit, mode),
        3 => form::<3>(register_bit, mode),
        4 => form::<4>(register_bit, mode),
        5 => form::<5>(register_bit, mode),
        6 => form::<6>(register_bit, mode),
        7 => form::<7>(register_bit, mode),
        8 => form::<8>(register_bit, mode),
        20 => form::<20>(register_bit, mode),
        21 => form::<21>(register_bit, mode),
        22 => form::<22>(register_bit, mode),
        23 => form::<23>(register_bit, mode),
        _ => form::<24>(register_bit, mode),
    }
}
/// `e.y` is the bit number, or a byte register when `REG_BIT`; the operation
/// is fixed by the handler; an absolute address occupies `e.imm` bits 8..24.
#[inline(always)]
fn bit_number<const REG_BIT: bool>(cpu: &Registers, e: &Entry) -> u8 {
    if REG_BIT {
        cpu.read(Size::Byte, e.y) as u8 & 7
    } else {
        e.y
    }
}
fn bit_register<const REG_BIT: bool, const OP: u32>(cpu: &mut Registers, e: &Entry) {
    let bit = bit_number::<REG_BIT>(cpu, e);
    let r = &mut *cpu;
    let value = r.read(Size::Byte, e.x) as u8;
    let (result, writes) = alu::bit(&mut r.ccr, bit_op(OP), bit, value);
    if writes {
        r.write(Size::Byte, e.x, u32::from(result));
    }
}
/// `@ERn` (`e.x`) when not `ABSOLUTE`, else `@aa:8` with the address in `e.imm`.
fn bit_memory<const REG_BIT: bool, const ABSOLUTE: bool, const OP: u32>(
    cpu: &mut Registers,
    lanes: &mut Lanes<'_, '_>,
    e: &Entry,
) -> Flow {
    let bit = bit_number::<REG_BIT>(cpu, e);
    let a = if ABSOLUTE {
        (e.imm >> 8) as u16
    } else {
        cpu.er[usize::from(e.x & 7)] as u16
    };
    let op = bit_op(OP);
    let writes = bit_writes(op);
    // A read-modify-write of a register is the interval bus's.
    if writes && !lanes.writable(a, Width::Byte) {
        return Flow::Decline;
    }
    let value = if writes {
        lanes.read(a, Width::Byte)
    } else {
        // After the instruction's extension words.
        lanes.data_read(a, Width::Byte, 2 * (e.len - 1))
    };
    let Some(value) = value else {
        return Flow::Decline;
    };
    let (result, _) = alu::bit(&mut cpu.ccr, op, bit, value as u8);
    if writes {
        lanes.write(a, Width::Byte, u16::from(result));
    }
    Flow::Next
}
/// Address modes of `transfer`: absolute, @ERn, @(d,ERn), @ERn+, @-ERn.
fn transfer<const SIZE: u8, const STORE: bool, const MODE: u8, const CCR: bool>(
    cpu: &mut Registers,
    lanes: &mut Lanes<'_, '_>,
    e: &Entry,
) -> Flow {
    let size = size_of(SIZE);
    let bytes = u32::from(size.bytes());
    let r = &mut *cpu;
    let base = usize::from(e.y & 7);
    // As `target_address`: @-ERn updates the register before the source is
    // read, and @ERn+ commits after the access.
    let old = r.er[base];
    let mut a = match MODE {
        0 => e.imm as u16,
        1 | 3 => old as u16,
        2 => old.wrapping_add(e.imm) as u16,
        _ => {
            r.er[base] = old.wrapping_sub(bytes);
            old.wrapping_sub(bytes) as u16
        }
    };
    let value = if CCR {
        u32::from(r.ccr) << 8
    } else {
        r.read(size, e.x)
    };
    if size != Size::Byte {
        a &= !1;
    }
    let width = if size == Size::Byte {
        Width::Byte
    } else {
        Width::Word
    };
    let count = if size == Size::Long { 2 } else { 1 };
    let mut loaded = 0u32;
    // The data lanes come last: all states before them are the offset.
    let offset = e.states - 2 * count as u8;
    if count == 2 {
        let ok = if STORE {
            lanes.writable(a, width) && lanes.writable(a.wrapping_add(2), width)
        } else {
            match (lanes.read(a, width), lanes.read(a.wrapping_add(2), width)) {
                (Some(high), Some(low)) => {
                    loaded = u32::from(high) << 16 | u32::from(low);
                    true
                }
                _ => false,
            }
        };
        if !ok {
            if MODE == 4 {
                cpu.er[base] = old;
            }
            return Flow::Decline;
        }
        if STORE {
            lanes.write(a, width, (value >> 16) as u16);
            lanes.write(a.wrapping_add(2), width, value as u16);
        }
    } else {
        let ok = if STORE {
            // As the timed MOV: a byte store's origin qualifies flag clears.
            let origin = match (size, CCR, MODE == 0 && e.len == 1) {
                (Size::Byte, false, true) => WriteOrigin::MovByteAbs8 {
                    pc_bit1: e.origin_pc_bit1,
                },
                (Size::Byte, false, false) => WriteOrigin::MovByte,
                _ => WriteOrigin::Other,
            };
            lanes.data_write(a, width, value as u16, origin, offset)
        } else {
            match lanes.data_read(a, width, offset) {
                Some(v) => {
                    loaded = u32::from(v);
                    true
                }
                None => false,
            }
        };
        if !ok {
            if MODE == 4 {
                cpu.er[base] = old;
            }
            return Flow::Decline;
        }
    }
    let r = &mut *cpu;
    if MODE == 3 {
        r.er[base] = old.wrapping_add(bytes);
    }
    if CCR {
        if !STORE {
            r.ccr = (loaded >> 8) as u8;
            return Flow::Defer;
        }
    } else {
        let value = if STORE {
            value
        } else {
            r.write(size, e.x, loaded);
            loaded
        };
        r.ccr = alu::logical(value, size, r.ccr);
    }
    Flow::Next
}

/// BSR, JSR and JMP: relative, absolute, register or vector target.
fn jump<const MODE: u8, const CALL: bool>(
    cpu: &mut Registers,
    lanes: &mut Lanes<'_, '_>,
    e: &Entry,
) -> Flow {
    let next = cpu.pc;
    let target = match MODE {
        0 => next.wrapping_add(e.imm as u16),
        1 => e.imm as u16,
        2 => cpu.er[usize::from(e.y & 7)] as u16,
        _ => match lanes.read(e.imm as u16 & !1, Width::Word) {
            Some(target) => target,
            None => return Flow::Decline,
        },
    };
    if lanes.read(target & !1, Width::Word).is_none() {
        return Flow::Decline;
    }
    if CALL {
        let sp = cpu.er[7].wrapping_sub(2);
        // The target's first word is already fetched when the push lands.
        if !lanes.writable(sp as u16 & !1, Width::Word) || sp as u16 & !1 == target & !1 {
            return Flow::Decline;
        }
        lanes.write(sp as u16 & !1, Width::Word, next);
        cpu.er[7] = sp;
    }
    cpu.pc = target & !1;
    Flow::Next
}
fn ret<const EXCEPTION: bool>(cpu: &mut Registers, lanes: &mut Lanes<'_, '_>, _: &Entry) -> Flow {
    let sp = cpu.er[7];
    let Some(first) = lanes.read(sp as u16 & !1, Width::Word) else {
        return Flow::Decline;
    };
    let target = if EXCEPTION {
        match lanes.read((sp as u16).wrapping_add(2) & !1, Width::Word) {
            Some(target) => target,
            None => return Flow::Decline,
        }
    } else {
        first
    };
    if lanes.read(target & !1, Width::Word).is_none() {
        return Flow::Decline;
    }
    let r = &mut *cpu;
    r.er[7] = sp.wrapping_add(if EXCEPTION { 4 } else { 2 });
    if EXCEPTION {
        // RTE is not one of §3.8.5's CCR-writing deferral instructions.
        r.ccr = (first >> 8) as u8;
    }
    r.pc = target & !1;
    Flow::Next
}
/// LDC/ANDC/ORC/XORC from `e.y` or, when `IMM`, `e.imm`.
fn ccr<const OP: u8, const IMM: bool>(
    cpu: &mut Registers,
    _: &mut Lanes<'_, '_>,
    e: &Entry,
) -> Flow {
    let r = &mut *cpu;
    let v = if IMM {
        e.imm as u8
    } else {
        r.read(Size::Byte, e.y) as u8
    };
    r.ccr = match OP {
        0 => v,
        1 => r.ccr & v,
        2 => r.ccr | v,
        _ => r.ccr ^ v,
    };
    Flow::Defer
}
fn store_ccr(cpu: &mut Registers, e: &Entry) {
    let r = &mut *cpu;
    let ccr = u32::from(r.ccr);
    r.write(Size::Byte, e.x, ccr);
}
/// MULXU/MULXS/DIVXU/DIVXS; `e.imm` holds divide, signed and the size code.
fn muldiv(cpu: &mut Registers, e: &Entry) {
    let (out_size, value, flags) = cpu.muldiv(
        e.imm & 1 != 0,
        e.imm & 2 != 0,
        size_of((e.imm >> 2) as u8),
        e.y,
        e.x,
    );
    let r = &mut *cpu;
    r.write(out_size, e.x, value);
    r.ccr = flags;
}
fn decimal<const SUBTRACT: bool>(cpu: &mut Registers, e: &Entry) {
    let r = &mut *cpu;
    let (v, f) = super::decimal(r.read(Size::Byte, e.x) as u8, SUBTRACT, r.ccr);
    r.write(Size::Byte, e.x, u32::from(v));
    r.ccr = f;
}

fn binary_handler(op: Alu, size: Size, imm: bool) -> PureHandler {
    macro_rules! pick {
        ($($op:ident),*) => {
            match (op, imm) {
                $(
                    (Alu::$op, false) => pick!(@size $op, false),
                    (Alu::$op, true) => pick!(@size $op, true),
                )*
            }
        };
        (@size $op:ident, $imm:expr) => {
            match size {
                Size::Byte => binary::<{ alu_code(Alu::$op) }, 0, $imm> as PureHandler,
                Size::Word => binary::<{ alu_code(Alu::$op) }, 1, $imm> as PureHandler,
                Size::Long => binary::<{ alu_code(Alu::$op) }, 2, $imm> as PureHandler,
            }
        };
    }
    pick!(Mov, Add, Addx, Sub, Subx, Cmp, And, Or, Xor)
}
macro_rules! sized {
    ($f:ident, $op:literal, $size:expr) => {
        match $size {
            Size::Byte => $f::<$op, 0> as PureHandler,
            Size::Word => $f::<$op, 1> as PureHandler,
            Size::Long => $f::<$op, 2> as PureHandler,
        }
    };
}
fn unary_handler(op: decode::Unary, size: Size) -> PureHandler {
    use decode::Unary::*;
    match op {
        Not => sized!(unary, 0, size),
        Neg => sized!(unary, 1, size),
        Inc => sized!(unary, 2, size),
        Dec => sized!(unary, 3, size),
        Extu => sized!(unary, 4, size),
        Exts => sized!(unary, 5, size),
    }
}
fn shift_handler(op: decode::Shift, size: Size) -> PureHandler {
    use decode::Shift::*;
    match op {
        Shll => sized!(shift, 0, size),
        Shal => sized!(shift, 1, size),
        Shlr => sized!(shift, 2, size),
        Shar => sized!(shift, 3, size),
        Rotl => sized!(shift, 4, size),
        Rotr => sized!(shift, 5, size),
        Rotxl => sized!(shift, 6, size),
        Rotxr => sized!(shift, 7, size),
    }
}
fn transfer_handler(size: Size, store: bool, mode: u8, ccr: bool) -> BusHandler {
    macro_rules! modes {
        ($store:expr, $ccr:expr) => {
            match mode {
                0 => sized_t!($store, 0, $ccr),
                1 => sized_t!($store, 1, $ccr),
                2 => sized_t!($store, 2, $ccr),
                3 => sized_t!($store, 3, $ccr),
                _ => sized_t!($store, 4, $ccr),
            }
        };
    }
    macro_rules! sized_t {
        ($store:expr, $mode:expr, $ccr:expr) => {
            match size {
                Size::Byte => transfer::<0, $store, $mode, $ccr> as BusHandler,
                Size::Word => transfer::<1, $store, $mode, $ccr> as BusHandler,
                Size::Long => transfer::<2, $store, $mode, $ccr> as BusHandler,
            }
        };
    }
    match (store, ccr) {
        (false, false) => modes!(false, false),
        (true, false) => modes!(true, false),
        (false, true) => modes!(false, true),
        (true, true) => modes!(true, true),
    }
}

/// The cache entry of an instruction, or `None` when the interval bus must
/// perform it.
fn entry(instruction: Instruction, words: [u16; 5], len: usize, generation: u64) -> Option<Entry> {
    let (states, slack, reads, writes) = timing(&instruction, len)?;
    let mut e = Entry {
        handler: Handler::Pure(nop),
        run: Run::EMPTY,
        origin_pc_bit1: false,
        target_word: None,
        x: 0,
        y: 0,
        len: len as u8,
        states,
        cost: states + slack * REGISTER_EXTRA,
        accesses: u64::from(reads) | u64::from(writes) << 32,
        imm: 0,
        generation,
        words,
    };
    use Instruction::*;
    e.handler = match instruction {
        Nop => Handler::Pure(nop),
        Binary { op, size, dst, src } => Handler::Pure({
            e.x = dst;
            match src {
                Source::Reg(s) => {
                    e.y = s;
                    binary_handler(op, size, false)
                }
                Source::Imm(v) => {
                    e.imm = v;
                    binary_handler(op, size, true)
                }
            }
        }),
        Unary {
            op,
            size,
            dst,
            amount,
        } => Handler::Pure({
            e.x = dst;
            e.imm = amount;
            unary_handler(op, size)
        }),
        Shift { op, size, dst } => Handler::Pure({
            e.x = dst;
            shift_handler(op, size)
        }),
        Quick { dst, delta } => Handler::Pure({
            e.x = dst;
            e.imm = i32::from(delta) as u32;
            quick
        }),
        Bit { op, bit, target } => {
            let reg_bit = match bit {
                BitIndex::Reg(b) => {
                    e.y = b;
                    true
                }
                BitIndex::Imm(b) => {
                    e.y = b;
                    false
                }
            };
            let mode = match target {
                Target::Reg(d) => {
                    e.x = d;
                    0
                }
                Target::Memory(Address::Indirect(b)) => {
                    e.x = b;
                    1
                }
                Target::Memory(Address::Absolute(a)) => {
                    e.imm = u32::from(a) << 8;
                    2
                }
                _ => return None,
            };
            bit_handler(bit_code(op), reg_bit, mode)
        }
        Memory {
            size,
            reg,
            address,
            store,
            ccr,
        } => {
            e.x = reg;
            let mode = match address {
                Address::Absolute(a) => {
                    e.imm = u32::from(a);
                    0
                }
                Address::Indirect(b) => {
                    e.y = b;
                    1
                }
                Address::Displaced { reg: b, offset } => {
                    e.y = b;
                    e.imm = offset as u32;
                    2
                }
                Address::PostIncrement(b) => {
                    e.y = b;
                    3
                }
                Address::PreDecrement(b) => {
                    e.y = b;
                    4
                }
            };
            if size == Size::Byte && mode == 0 && !ccr {
                if store {
                    Handler::WriteByteAbsolute
                } else {
                    Handler::ReadByteAbsolute
                }
            } else {
                Handler::Bus(transfer_handler(size, store, mode, ccr))
            }
        }
        Branch { condition, .. } => {
            // The displacement becomes a qualified absolute target in decode_at.
            let truth = (0u8..16).fold(0u16, |bits, flags| {
                bits | u16::from(alu::condition(condition, flags)) << flags
            });
            e.imm = u32::from(truth) << 16;
            Handler::Branch
        }
        BranchSubroutine(displacement) => Handler::Bus({
            e.imm = displacement as u32;
            jump::<0, true>
        }),
        Jump { target, call } => Handler::Bus(match target {
            decode::Jump::Absolute(a) => {
                e.imm = u32::from(a);
                if call {
                    jump::<1, true>
                } else {
                    jump::<1, false>
                }
            }
            decode::Jump::Register(b) => {
                e.y = b;
                if call {
                    jump::<2, true>
                } else {
                    jump::<2, false>
                }
            }
            decode::Jump::Vector(a) => {
                e.imm = u32::from(a);
                if call {
                    jump::<3, true>
                } else {
                    jump::<3, false>
                }
            }
        }),
        Return { exception } => Handler::Bus({
            if exception {
                ret::<true>
            } else {
                ret::<false>
            }
        }),
        Ccr { op, source } => Handler::Bus({
            let imm = match source {
                Source::Reg(s) => {
                    e.y = s;
                    false
                }
                Source::Imm(v) => {
                    e.imm = v & 255;
                    true
                }
            };
            match (op, imm) {
                (CcrOp::Load, false) => ccr::<0, false>,
                (CcrOp::Load, true) => ccr::<0, true>,
                (CcrOp::And, false) => ccr::<1, false>,
                (CcrOp::And, true) => ccr::<1, true>,
                (CcrOp::Or, false) => ccr::<2, false>,
                (CcrOp::Or, true) => ccr::<2, true>,
                (CcrOp::Xor, false) => ccr::<3, false>,
                (CcrOp::Xor, true) => ccr::<3, true>,
            }
        }),
        StoreCcr(d) => Handler::Pure({
            e.x = d;
            store_ccr
        }),
        MulDiv {
            divide,
            signed,
            size,
            src,
            dst,
        } => Handler::Pure({
            e.x = dst;
            e.y = src;
            e.imm = u32::from(divide) | u32::from(signed) << 1 | u32::from(size_code(size)) << 2;
            muldiv
        }),
        Decimal { subtract, reg } => Handler::Pure({
            e.x = reg;
            if subtract {
                decimal::<true>
            } else {
                decimal::<false>
            }
        }),
        Sleep | Trap(_) | EepMov { .. } => return None,
    };
    Some(e)
}

/// The instruction word at `pc` in flash or RAM.
fn code_word(memory: &Memory<'_>, pc: u16) -> Option<u16> {
    let i = usize::from(pc);
    if i + 1 < FLASH_SIZE {
        return Some(u16::from_be_bytes([memory.flash[i], memory.flash[i + 1]]));
    }
    if (RAM_START..0xff7f).contains(&pc) {
        let i = usize::from(pc - RAM_START);
        return Some(u16::from_be_bytes([memory.ram[i], memory.ram[i + 1]]));
    }
    None
}
/// Decode the instruction at `pc` from memory.
fn decode_at(memory: &Memory<'_>, pc: u16, generation: u64) -> Option<Entry> {
    let mut words = [0u16; 5];
    let mut count = 0;
    let instruction = loop {
        if count == words.len() {
            return None;
        }
        words[count] = code_word(memory, pc.wrapping_add(2 * count as u16))?;
        count += 1;
        match decode::decode(&words[..count]) {
            Decode::Ready { instruction, words } if usize::from(words) == count => {
                break instruction
            }
            Decode::NeedWord => {}
            _ => return None,
        }
    };
    // Even a discarded NEXT uses the physical bus. Keep instructions at a
    // memory region's end on the timed bus, including their split fetches.
    code_word(memory, pc.wrapping_add(2 * count as u16))?;
    let mut decoded = entry(instruction, words, count, generation)?;
    decoded.origin_pc_bit1 = pc & 2 != 0;
    if let Instruction::Branch { displacement, .. } = instruction {
        let target = pc
            .wrapping_add(2 * count as u16)
            .wrapping_add(displacement as u16)
            & !1;
        // Both possible fetches have no side effects inside the interval.
        // Their timing/access counts still come from the canonical sequence.
        let word = code_word(memory, target)?;
        // RAM entries validate their own words, not the target's contents.
        // Only the flash table's generation guard permits retaining this word.
        if usize::from(pc) < FLASH_SIZE && usize::from(target) < FLASH_SIZE {
            decoded.target_word = Some(word);
        }
        decoded.imm |= u32::from(target);
        decoded.y = u8::from(target <= pc);
    }
    Some(decoded)
}
