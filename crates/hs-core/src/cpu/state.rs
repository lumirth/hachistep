//! Saved CPU work, independent of executor layout.
use super::*;
use crate::state::require;
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshSerialize, BorshDeserialize)]
pub(crate) struct SavedCpu {
    registers: Registers,
    prefetch: Option<(u16, u16)>,
    interrupt_deferral: u8,
    instruction_address: u16,
    exception_notification: Option<u8>,
    progress: Progress,
    /// Results of the physical operations the unfinished work already completed.
    completed: Vec<u16>,
}
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Progress {
    InstructionBoundary = 0,
    Sleeping = 1,
    ResetEntry = 2,
    Instruction = 3,
    ExceptionEntry {
        vector: u8,
        preceded_by: Preceding,
        saved_pc: u16,
        saved_ccr: u8,
    } = 4,
    CopyPairs {
        word_count: bool,
    } = 5,
}
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Preceding {
    DiscardedFetch = 0,
    Trap = 1,
    Sleep = 2,
}
impl Cpu {
    pub(crate) fn save(&self) -> Result<SavedCpu, Error> {
        let progress = match self.state {
            State::Boundary => Progress::InstructionBoundary,
            State::Sleeping => Progress::Sleeping,
            State::Busy(Work::Reset) => Progress::ResetEntry,
            State::Busy(Work::Instruction) => Progress::Instruction,
            State::Busy(Work::Exception {
                vector,
                entry,
                pc,
                ccr,
            }) => Progress::ExceptionEntry {
                vector,
                preceded_by: match entry {
                    Entry::Fetch => Preceding::DiscardedFetch,
                    Entry::Trap => Preceding::Trap,
                    Entry::Sleep => Preceding::Sleep,
                },
                saved_pc: pc,
                saved_ccr: ccr,
            },
            State::Busy(Work::Copy { word_count }) => Progress::CopyPairs { word_count },
        };
        Ok(SavedCpu {
            registers: self.registers,
            prefetch: self.prefetch.get(),
            interrupt_deferral: self.interrupt_delay,
            instruction_address: self.instruction_pc,
            exception_notification: self.accepted_vector,
            progress,
            completed: self.completed.values[..usize::from(self.completed.count)].to_vec(),
        })
    }
}
impl SavedCpu {
    pub(crate) fn restore(self, faulted: bool) -> Result<Cpu, Error> {
        require(
            self.interrupt_deferral <= 1
                && self.registers.pc & 1 == 0
                && self.instruction_address & 1 == 0
                && self.prefetch.is_none_or(|(a, _)| a & 1 == 0)
                && self.exception_notification.is_none_or(|v| v < 64)
                && self.completed.len() <= COMPLETED,
            "invalid saved CPU state",
        )?;
        let state = match self.progress {
            Progress::InstructionBoundary => State::Boundary,
            Progress::Sleeping => State::Sleeping,
            Progress::ResetEntry => State::Busy(Work::Reset),
            Progress::Instruction => State::Busy(Work::Instruction),
            Progress::ExceptionEntry {
                vector,
                preceded_by,
                saved_pc,
                saved_ccr,
            } => {
                require(vector < 64 && saved_pc & 1 == 0, "invalid saved exception")?;
                State::Busy(Work::Exception {
                    vector,
                    entry: match preceded_by {
                        Preceding::DiscardedFetch => Entry::Fetch,
                        Preceding::Trap => Entry::Trap,
                        Preceding::Sleep => Entry::Sleep,
                    },
                    pc: saved_pc,
                    ccr: saved_ccr,
                })
            }
            Progress::CopyPairs { word_count } => {
                require(
                    Cpu::copy_count(&self.registers, word_count) != 0,
                    "copy without remaining bytes",
                )?;
                State::Busy(Work::Copy { word_count })
            }
        };
        require(
            matches!(state, State::Busy(_)) || self.completed.is_empty(),
            "completed accesses without unfinished work",
        )?;
        let mut values = [0; COMPLETED];
        values[..self.completed.len()].copy_from_slice(&self.completed);
        let cpu = Cpu {
            registers: self.registers,
            state,
            prefetch: Prefetch::from_saved(self.prefetch),
            interrupt_delay: self.interrupt_deferral,
            instruction_pc: self.instruction_address,
            accepted_vector: self.exception_notification,
            origin: Origin::new(
                &self.registers,
                Prefetch::from_saved(self.prefetch),
                self.interrupt_deferral,
            ),
            completed: Completed {
                values,
                count: self.completed.len() as u8,
                replayed: 0,
            },
            memo: memo::DecodeMemo::empty(),
            code: direct::Code::empty(),
            repetition: repetition::Repetition::new(),
            retired: 0,
            interrupt_entries: 0,
        };
        // Replaying the completed accesses must leave the work unfinished: at
        // a further physical operation or an admission point.
        if matches!(state, State::Busy(_)) {
            let mut probe = cpu.clone();
            match probe.execute(&mut Projection {
                interrupt: None,
                admit: false,
            }) {
                Err(Stop::Request(_)) | Err(Stop::Sample) => {}
                Err(Stop::Core(_)) if faulted => {}
                _ => return Err(Error::Snapshot("saved CPU progress is not unfinished work")),
            }
        }
        Ok(cpu)
    }
}
