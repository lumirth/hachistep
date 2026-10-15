//! Decoded instructions remembered by address. A memo is not state: every
//! use is validated against the words the CPU actually fetched, so guest
//! writes to code need no invalidation, and copies start empty.
use super::decode::{Instruction, Jump, Target};

/// What an instruction does on the bus between its fetch and its operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Prelude {
    /// Memory bit operations and EEPMOV fetch NEXT later in their own
    /// sequences; JMP @aa:16 spends its address interval first.
    None,
    /// Bcc/BSR d:16 calculate the branch address instead (§2.8, pp.235–236).
    Calculate,
    /// NEXT is fetched and discarded by instructions that change flow.
    Fetch,
    /// NEXT is fetched and retained for the following instruction.
    Retain,
}
impl Prelude {
    pub fn of(instruction: &Instruction, count: usize) -> Self {
        match instruction {
            Instruction::Bit {
                target: Target::Memory(_),
                ..
            }
            | Instruction::EepMov { .. }
            | Instruction::Jump {
                target: Jump::Absolute(_),
                ..
            } => Self::None,
            Instruction::Branch { .. } | Instruction::BranchSubroutine(_) if count == 2 => {
                Self::Calculate
            }
            Instruction::Jump { .. }
            | Instruction::BranchSubroutine(_)
            | Instruction::Return { .. }
            | Instruction::Trap(_) => Self::Fetch,
            _ => Self::Retain,
        }
    }
}

const ENTRIES: usize = 4096;
#[derive(Clone, Copy)]
struct Entry {
    pc: u16,
    /// Zero for an empty entry.
    count: u8,
    prelude: Prelude,
    words: [u16; 5],
    instruction: Instruction,
}
pub(crate) struct DecodeMemo {
    entries: Option<Box<[Entry; ENTRIES]>>,
}
impl DecodeMemo {
    pub fn empty() -> Self {
        Self { entries: None }
    }
    pub fn prepare(&mut self) {
        self.entries.get_or_insert_with(|| {
            Box::new(
                [Entry {
                    pc: 0,
                    count: 0,
                    prelude: Prelude::None,
                    words: [0; 5],
                    instruction: Instruction::Nop,
                }; ENTRIES],
            )
        });
    }
    /// The instruction remembered at `pc` beginning with `first`, and the
    /// words that must follow it.
    #[inline(always)]
    pub fn lookup(&self, pc: u16, first: u16) -> Option<([u16; 5], usize, Instruction, Prelude)> {
        let entry = &self.entries.as_ref()?[usize::from(pc >> 1) % ENTRIES];
        (entry.pc == pc && entry.count != 0 && entry.words[0] == first).then_some((
            entry.words,
            usize::from(entry.count),
            entry.instruction,
            entry.prelude,
        ))
    }
    pub fn remember(&mut self, pc: u16, words: &[u16], instruction: Instruction, prelude: Prelude) {
        let Some(entries) = &mut self.entries else {
            return;
        };
        let mut entry = Entry {
            pc,
            count: words.len() as u8,
            prelude,
            words: [0; 5],
            instruction,
        };
        entry.words[..words.len()].copy_from_slice(words);
        entries[usize::from(pc >> 1) % ENTRIES] = entry;
    }
}
impl Clone for DecodeMemo {
    fn clone(&self) -> Self {
        Self::empty()
    }
}
impl PartialEq for DecodeMemo {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for DecodeMemo {}
impl core::fmt::Debug for DecodeMemo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DecodeMemo")
    }
}
