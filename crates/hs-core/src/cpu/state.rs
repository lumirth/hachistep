//! Saved hardware work, independent of executor continuation identifiers.
//! Tag meanings and completed effects: docs/research/save-state-cpu-progress.md.
use super::*;
use crate::state::require;
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshSerialize, BorshDeserialize)]
pub(crate) struct SavedCpu {
    registers: Registers,
    instruction_address: u16,
    fetched_count: u8,
    fetched_words: [u16; 5],
    prefetch: Option<(u16, u16)>,
    interrupt_deferral: u8,
    exception_notification: Option<u8>,
    progress: Progress,
}
#[derive(BorshSerialize, BorshDeserialize)]
struct Frame {
    vector: u8,
    pc: u16,
    ccr: u8,
}
impl Frame {
    fn phase(self) -> Result<Phase, Error> {
        require(self.vector < 64, "invalid saved exception vector")?;
        Ok(Phase::ExceptionPc {
            vector: self.vector,
            pc: self.pc,
            ccr: self.ccr,
        })
    }
}
#[derive(BorshSerialize, BorshDeserialize)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum CopyStep {
    ReadInitialSource = 0,
    ReadInitialDestination = 1,
    AdmitNextByte = 2,
    ReadAdmittedByte = 3,
    WriteLatchedByte(u8) = 4,
}
#[derive(BorshSerialize, BorshDeserialize)]
#[repr(u8)]
#[borsh(use_discriminant = true)]
enum Progress {
    ReadResetVector = 0,
    InstructionBoundary = 1,
    FetchInstructionWord = 2,
    PrepareFetchedInstruction = 3,
    ExecuteFetchedInstruction = 4,
    FetchNextBeforeExecution {
        address: u16,
        retain: bool,
    } = 5,
    FetchDiscardBeforeException {
        address: u16,
        frame: Frame,
    } = 6,
    FetchNextBeforeBitWrite {
        address: u16,
        write_address: u16,
        result: u8,
    } = 7,
    FetchNextBeforeRetirement {
        address: u16,
    } = 8,
    CalculateBranchAddress = 9,
    DelayAfterAddressUpdate(Transfer) = 10,
    DelayBeforeExceptionStack {
        states: u32,
        frame: Frame,
    } = 11,
    RetireInstruction = 12,
    TransferMemory(Transfer) = 13,
    ReadBitOperand {
        address: u16,
        operation: Bit,
        bit: u8,
    } = 14,
    WriteBitResult {
        address: u16,
        result: u8,
    } = 15,
    ReadIndirectTarget {
        address: u16,
        call: bool,
    } = 16,
    WriteReturnBeforeTargetFetch {
        address: u16,
        target: u16,
        return_pc: u16,
    } = 17,
    WriteReturnAfterTargetFetch {
        address: u16,
        return_pc: u16,
    } = 18,
    FetchConditionalTarget {
        target: u16,
        install: bool,
    } = 19,
    FetchJumpTarget {
        target: u16,
        stack_return: Option<u16>,
    } = 20,
    DelayBeforeJumpTarget {
        target: u16,
        call: bool,
    } = 21,
    DelayBeforeEntryTarget {
        target: u16,
    } = 22,
    FetchEntryTarget {
        target: u16,
    } = 23,
    ReadReturnPc = 24,
    ReadReturnCcr = 25,
    ReadExceptionReturnPc {
        pending_ccr: u8,
    } = 26,
    WriteExceptionPc(Frame) = 27,
    WriteExceptionCcr {
        vector: u8,
        saved_ccr: u8,
    } = 28,
    ReadExceptionVector {
        vector: u8,
    } = 29,
    ArithmeticResultPending {
        result_size: Size,
        destination: u8,
        result: u32,
        pending_ccr: u8,
        states: u32,
    } = 30,
    CopyBytes {
        word_count: bool,
        step: CopyStep,
    } = 31,
    Sleeping = 32,
}
impl Cpu {
    pub(crate) fn save(&self) -> Result<SavedCpu, Error> {
        use Progress::*;
        let progress = match self.phase {
            Phase::ResetVector => ReadResetVector,
            Phase::Boundary => InstructionBoundary,
            Phase::Fetch => FetchInstructionWord,
            Phase::Ready(_) => PrepareFetchedInstruction,
            Phase::Execute(_) => ExecuteFetchedInstruction,
            Phase::Prefetch { address, retain } => match self.continuation {
                Phase::Execute(_) => FetchNextBeforeExecution { address, retain },
                Phase::ExceptionPc { vector, pc, ccr } if !retain => FetchDiscardBeforeException {
                    address,
                    frame: Frame { vector, pc, ccr },
                },
                Phase::BitWrite {
                    address: write_address,
                    value,
                } if retain => FetchNextBeforeBitWrite {
                    address,
                    write_address,
                    result: value,
                },
                Phase::Finish if retain => FetchNextBeforeRetirement { address },
                _ => return Err(Error::Snapshot("unrepresented prefetch work")),
            },
            Phase::Delay(states) => match self.continuation {
                Phase::Execute(_) if states == 2 => CalculateBranchAddress,
                Phase::Memory(t) if states == 2 => DelayAfterAddressUpdate(t),
                Phase::ExceptionPc { vector, pc, ccr } => DelayBeforeExceptionStack {
                    states,
                    frame: Frame { vector, pc, ccr },
                },
                _ => return Err(Error::Snapshot("unrepresented internal work")),
            },
            Phase::Finish => RetireInstruction,
            Phase::Memory(t) => TransferMemory(t),
            Phase::BitRead { address, op, bit } => ReadBitOperand {
                address,
                operation: op,
                bit,
            },
            Phase::BitWrite { address, value } => WriteBitResult {
                address,
                result: value,
            },
            Phase::IndirectJump { address, call } => ReadIndirectTarget { address, call },
            Phase::Call {
                address,
                target,
                return_pc,
                fetch_target: true,
            } => WriteReturnBeforeTargetFetch {
                address,
                target,
                return_pc,
            },
            Phase::Call {
                address,
                return_pc,
                fetch_target: false,
                ..
            } => WriteReturnAfterTargetFetch { address, return_pc },
            Phase::BranchTarget { target, take } => FetchConditionalTarget {
                target,
                install: take,
            },
            Phase::JumpTarget {
                target,
                call,
                return_pc,
            } => FetchJumpTarget {
                target,
                stack_return: call.then_some(return_pc),
            },
            Phase::BranchWait { target, call } => DelayBeforeJumpTarget { target, call },
            Phase::EntryWait { target } => DelayBeforeEntryTarget { target },
            Phase::EntryTarget { target } => FetchEntryTarget { target },
            Phase::ReturnPc => ReadReturnPc,
            Phase::ReturnCcr => ReadReturnCcr,
            Phase::ReturnExceptionPc { ccr } => ReadExceptionReturnPc { pending_ccr: ccr },
            Phase::ExceptionPc { vector, pc, ccr } => WriteExceptionPc(Frame { vector, pc, ccr }),
            Phase::ExceptionCcr { vector, ccr } => WriteExceptionCcr {
                vector,
                saved_ccr: ccr,
            },
            Phase::ExceptionVector { vector } => ReadExceptionVector { vector },
            Phase::MulDiv {
                size,
                dst,
                value,
                flags,
                states,
            } => ArithmeticResultPending {
                result_size: size,
                destination: dst,
                result: value,
                pending_ccr: flags,
                states,
            },
            Phase::Copy {
                word_count,
                stage,
                value,
            } => CopyBytes {
                word_count,
                step: match stage {
                    0 => CopyStep::ReadInitialSource,
                    1 => CopyStep::ReadInitialDestination,
                    2 => CopyStep::AdmitNextByte,
                    4 => CopyStep::ReadAdmittedByte,
                    3 => CopyStep::WriteLatchedByte(value),
                    _ => return Err(Error::Snapshot("invalid copy progress")),
                },
            },
            Phase::Sleeping => Sleeping,
        };
        let mut fetched_words = self.words;
        require(self.word_count <= 5, "invalid fetched word count")?;
        fetched_words[usize::from(self.word_count)..].fill(0);
        Ok(SavedCpu {
            registers: self.registers.clone(),
            instruction_address: self.instruction_pc,
            fetched_count: self.word_count,
            fetched_words,
            prefetch: self.prefetch,
            interrupt_deferral: self.interrupt_delay,
            exception_notification: self.accepted_vector,
            progress,
        })
    }
}
impl SavedCpu {
    pub(crate) fn restore(self, faulted: bool) -> Result<Cpu, Error> {
        require(
            self.fetched_count <= 5
                && self.interrupt_deferral <= 1
                && self.registers.pc & 1 == 0
                && self.instruction_address & 1 == 0
                && self.prefetch.is_none_or(|(a, _)| a & 1 == 0)
                && self.exception_notification.is_none_or(|v| v < 64)
                && self.fetched_words[usize::from(self.fetched_count)..]
                    .iter()
                    .all(|v| *v == 0),
            "invalid saved CPU state",
        )?;
        let decoded = decode::decode(&self.fetched_words[..usize::from(self.fetched_count)]);
        let instruction = || match decoded {
            Decode::Ready { instruction, words } if words == self.fetched_count => Ok(instruction),
            _ => Err(Error::Snapshot(
                "saved progress requires a complete instruction",
            )),
        };
        let mut continuation = Phase::Boundary;
        use Progress::*;
        let phase = match self.progress {
            ReadResetVector => Phase::ResetVector,
            InstructionBoundary => Phase::Boundary,
            FetchInstructionWord => {
                require(
                    faulted || (self.fetched_count < 5 && decoded == Decode::NeedWord),
                    "invalid instruction fetch progress",
                )?;
                Phase::Fetch
            }
            PrepareFetchedInstruction => Phase::Ready(instruction()?),
            ExecuteFetchedInstruction => Phase::Execute(instruction()?),
            FetchNextBeforeExecution { address, retain } => {
                continuation = Phase::Execute(instruction()?);
                Phase::Prefetch { address, retain }
            }
            FetchDiscardBeforeException { address, frame } => {
                continuation = frame.phase()?;
                Phase::Prefetch {
                    address,
                    retain: false,
                }
            }
            FetchNextBeforeBitWrite {
                address,
                write_address,
                result,
            } => {
                continuation = Phase::BitWrite {
                    address: write_address,
                    value: result,
                };
                Phase::Prefetch {
                    address,
                    retain: true,
                }
            }
            FetchNextBeforeRetirement { address } => {
                continuation = Phase::Finish;
                Phase::Prefetch {
                    address,
                    retain: true,
                }
            }
            CalculateBranchAddress => {
                let i = instruction()?;
                require(
                    self.fetched_count == 2
                        && matches!(
                            i,
                            Instruction::Branch { .. } | Instruction::BranchSubroutine(_)
                        ),
                    "invalid branch calculation",
                )?;
                continuation = Phase::Execute(i);
                Phase::Delay(2)
            }
            DelayAfterAddressUpdate(t) => {
                t.validate(instruction()?, self.fetched_count)?;
                require(
                    t.done == 0
                        && matches!(
                            instruction()?,
                            Instruction::Memory {
                                address: Address::PreDecrement(_) | Address::PostIncrement(_),
                                ..
                            }
                        ),
                    "invalid address-update delay",
                )?;
                continuation = Phase::Memory(t);
                Phase::Delay(2)
            }
            DelayBeforeExceptionStack { states, frame } => {
                require(matches!(states, 2 | 4), "invalid exception interval")?;
                continuation = frame.phase()?;
                Phase::Delay(states)
            }
            RetireInstruction => Phase::Finish,
            TransferMemory(t) => {
                t.validate(instruction()?, self.fetched_count)?;
                Phase::Memory(t)
            }
            ReadBitOperand {
                address,
                operation,
                bit,
            } => {
                require(
                    bit < 8
                        && matches!(instruction()?, Instruction::Bit { op, target: Target::Memory(_), .. } if op == operation),
                    "invalid bit operand",
                )?;
                Phase::BitRead {
                    address,
                    op: operation,
                    bit,
                }
            }
            WriteBitResult { address, result } => Phase::BitWrite {
                address,
                value: result,
            },
            ReadIndirectTarget { address, call } => Phase::IndirectJump { address, call },
            WriteReturnBeforeTargetFetch {
                address,
                target,
                return_pc,
            } => Phase::Call {
                address,
                target,
                return_pc,
                fetch_target: true,
            },
            WriteReturnAfterTargetFetch { address, return_pc } => Phase::Call {
                address,
                target: 0,
                return_pc,
                fetch_target: false,
            },
            FetchConditionalTarget { target, install } => Phase::BranchTarget {
                target,
                take: install,
            },
            FetchJumpTarget {
                target,
                stack_return,
            } => Phase::JumpTarget {
                target,
                call: stack_return.is_some(),
                return_pc: stack_return.unwrap_or(0),
            },
            DelayBeforeJumpTarget { target, call } => Phase::BranchWait { target, call },
            DelayBeforeEntryTarget { target } => Phase::EntryWait { target },
            FetchEntryTarget { target } => Phase::EntryTarget { target },
            ReadReturnPc => Phase::ReturnPc,
            ReadReturnCcr => Phase::ReturnCcr,
            ReadExceptionReturnPc { pending_ccr } => Phase::ReturnExceptionPc { ccr: pending_ccr },
            WriteExceptionPc(frame) => frame.phase()?,
            WriteExceptionCcr { vector, saved_ccr } => {
                require(vector < 64, "invalid saved vector")?;
                Phase::ExceptionCcr {
                    vector,
                    ccr: saved_ccr,
                }
            }
            ReadExceptionVector { vector } => {
                require(vector < 64, "invalid saved vector")?;
                Phase::ExceptionVector { vector }
            }
            ArithmeticResultPending {
                result_size: size,
                destination: dst,
                result: value,
                pending_ccr: flags,
                states,
            } => {
                require(
                    matches!((size, states), (Size::Word, 12) | (Size::Long, 20))
                        && dst < if size == Size::Long { 8 } else { 16 }
                        && value & !size.mask() == 0
                        && matches!(instruction()?, Instruction::MulDiv { dst: d, size: s, .. } if d == dst && s.bytes() * 2 == size.bytes()),
                    "invalid arithmetic result",
                )?;
                Phase::MulDiv {
                    size,
                    dst,
                    value,
                    flags,
                    states,
                }
            }
            CopyBytes { word_count, step } => {
                require(
                    matches!(instruction()?, Instruction::EepMov { word_count: w } if w == word_count),
                    "invalid copy instruction",
                )?;
                let (stage, value) = match step {
                    CopyStep::ReadInitialSource => (0, 0),
                    CopyStep::ReadInitialDestination => (1, 0),
                    CopyStep::AdmitNextByte => (2, 0),
                    CopyStep::ReadAdmittedByte => (4, 0),
                    CopyStep::WriteLatchedByte(v) => (3, v),
                };
                require(
                    stage < 2
                        || self.registers.read(
                            if word_count { Size::Word } else { Size::Byte },
                            if word_count { 4 } else { 12 },
                        ) != 0,
                    "copy without remaining bytes",
                )?;
                Phase::Copy {
                    word_count,
                    stage,
                    value,
                }
            }
            Sleeping => Phase::Sleeping,
        };
        Ok(Cpu {
            registers: self.registers,
            phase,
            continuation,
            prefetch: self.prefetch,
            accepted_vector: self.exception_notification,
            words: self.fetched_words,
            word_count: self.fetched_count,
            instruction_pc: self.instruction_address,
            interrupt_delay: self.interrupt_deferral,
            retired: 0,
            interrupt_entries: 0,
        })
    }
}
impl Transfer {
    fn validate(self, instruction: Instruction, count: u8) -> Result<(), Error> {
        let Self {
            address,
            size,
            register,
            store,
            ccr,
            absolute8,
            value,
            done,
            post,
        } = self;
        require(
            register < if size == Size::Long { 8 } else { 16 }
                && (size == Size::Byte || address & 1 == 0)
                && (done == 0 || (size == Size::Long && done == 2))
                && post.is_none_or(|(r, _)| r < 8)
                && if store {
                    value & !size.mask() == 0
                } else {
                    value <= if done == 0 { 0 } else { 0xffff }
                },
            "invalid memory transfer",
        )?;
        require(
            matches!(instruction, Instruction::Memory { size: s, reg, address: a, store: wr, ccr: c }
            if s == size && reg == register && wr == store && c == ccr
            && absolute8 == (matches!(a, Address::Absolute(_)) && count == 1)
            && match a { Address::PostIncrement(r) => post.is_some_and(|(p, _)| p == r), _ => post.is_none() }),
            "transfer differs from fetched instruction",
        )
    }
}
