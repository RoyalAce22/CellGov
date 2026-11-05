//! The PPE interrupts a PPU thread can take, and CellGov's own stops.
//!
//! [CBE-Handbook p:240 s:9.2] Table 9-1 lists the interrupts an instruction can cause.
//! [CBE-Handbook p:238 s:9.2] On the PPE every one of them is precise.

use cellgov_exec::{DataAccessFault, InterruptRecord, LocalDiagnostics};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;

use crate::fault_codes::{
    kind_index, PpuFaultClass, FAULT_ALIGNMENT, FAULT_DATA_SEGMENT, FAULT_DATA_STORAGE,
    FAULT_DEBUG_BREAK, FAULT_FP_UNAVAILABLE, FAULT_INSTRUCTION_SEGMENT, FAULT_INSTRUCTION_STORAGE,
    FAULT_INTERNAL, FAULT_PROGRAM, FAULT_SYSTEM_CALL, FAULT_UNIMPLEMENTED_INSN,
    FAULT_VXU_UNAVAILABLE,
};
use crate::instruction::PpuInstructionKind;

/// Why a PPU thread stopped: a PPE interrupt, or a point where CellGov
/// cannot go on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PpuFault {
    /// The instruction causes a PPE interrupt.
    #[error(transparent)]
    Interrupt(#[from] PpuInterrupt),
    /// Decoded instruction with no execution arm.
    #[error("PPU unimplemented instruction: {mnemonic}")]
    UnimplementedInstruction {
        /// The decoded instruction's kind; the guest code carries its index.
        kind: PpuInstructionKind,
        /// The decoded instruction's mnemonic.
        mnemonic: &'static str,
    },
    /// A debug breakpoint fired at a user-requested PC.
    #[error("PPU debug break")]
    DebugBreak,
    /// A state the interpreter's own invariants exclude. The payload
    /// names the invariant.
    #[error("PPU internal stop: {0}")]
    Internal(&'static str),
}

/// A PPE interrupt that problem-state code causes, with its cause fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PpuInterrupt {
    /// A load or store whose address no region holds, or that a region
    /// refuses.
    ///
    /// An address no region holds takes this interrupt with no
    /// translation found. The console can take the data segment
    /// interrupt there instead. That choice depends on LV2's segment
    /// table, and CellGov does not model it.
    #[error("PPU data storage interrupt at EA 0x{ea:016x}")]
    DataStorage {
        /// The first byte of the refused access: the DAR.
        ea: u64,
        /// The access was a store, `dcbz` or conditional store.
        store: bool,
        /// Storage protection refused the access; otherwise the access
        /// found no translation.
        protection: bool,
    },
    /// An access whose segment has no entry. CellGov does not raise it:
    /// see [`Self::DataStorage`].
    #[error("PPU data segment interrupt at EA 0x{ea:016x}")]
    DataSegment {
        /// The first byte of the refused access: the DAR.
        ea: u64,
        /// The access was a store or `dcbz`.
        store: bool,
    },
    /// A fetch from an address no region holds.
    #[error("PPU instruction storage interrupt")]
    InstructionStorage,
    /// A fetch whose segment has no entry. CellGov does not raise it.
    #[error("PPU instruction segment interrupt")]
    InstructionSegment,
    /// An access the PPE refuses to perform at its address.
    #[error("PPU alignment interrupt at EA 0x{ea:016x}")]
    Alignment {
        /// The access's effective address: the DAR.
        ea: u64,
        /// The access was a store, `dcbz` or conditional store.
        store: bool,
    },
    /// A program interrupt and its one cause.
    #[error("PPU program interrupt: {0}")]
    Program(ProgramCause),
    /// A floating-point instruction with MSR\[FP\] = 0. CellGov does not
    /// raise it: a game process never sees it.
    ///
    /// [PPC-Book3 p:72 s:5.5.10] Floating-point unavailable needs MSR\[FP\] = 0, which only privileged code sets.
    #[error("PPU floating-point unavailable interrupt")]
    FpUnavailable,
    /// A vector instruction with MSR\[VXU\] = 0. CellGov does not raise
    /// it: a game process never sees it.
    ///
    /// SRR0 holds the vector instruction itself, because the processor
    /// attempts it next and it cannot run.
    ///
    /// [CBE-Handbook p:258 s:9.5.15] VXU unavailable needs MSR\[VXU\] = 0, and SRR0 holds the instruction the processor would have attempted to execute next.
    #[error("PPU VXU unavailable interrupt")]
    VxuUnavailable,
    /// `sc` with LEV = 1, which invokes the hypervisor. An `sc` with
    /// LEV = 0 is a syscall the unit yields.
    ///
    /// The thread stops, and LV2 receives no request. No document or
    /// console capture shows what LV1 does with this call from a game
    /// process.
    ///
    /// [PPC-Book3 p:12 s:2.3.1] `sc` with LEV = 1 invokes the hypervisor, even from problem state, and the architecture calls that a programming error.
    /// [CBE-Handbook p:256 s:9.5.13] On the PPE, LEV = 1 sets MSR\[HV\].
    #[error("PPU hypervisor call: sc with LEV = {lev} from problem state")]
    SystemCall {
        /// The `sc` instruction's LEV field.
        lev: u8,
    },
}

/// The cause of a program interrupt.
///
/// [CBE-Handbook p:254 s:9.5.9] The PPE takes every program-interrupt cause the architecture lists, plus a reserved SPR and the invalid forms of load and store with update, load multiple and load string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProgramCause {
    /// An enabled floating-point exception. CellGov does not raise it: a
    /// game process runs with MSR\[FE0\] = MSR\[FE1\] = 0, which ignores
    /// exceptions.
    #[error("floating-point enabled exception")]
    FpEnabled,
    /// An illegal instruction: an instruction the PPE does not
    /// implement, a reserved encoding, or an invalid form. The payload
    /// names the instruction.
    ///
    /// [PPC-Book1 p:13 s:1.9.2] An invalid form either invokes the system illegal instruction error handler or yields boundedly undefined results. CellGov takes the handler arm so both build profiles refuse the form the same way.
    /// [PPC-Book1 p:14 s:1.10] An optional instruction the implementation does not provide invokes the system illegal instruction error handler.
    #[error("illegal instruction {0}")]
    Illegal(&'static str),
    /// An instruction that needs privileged or hypervisor state. The
    /// payload names the instruction.
    #[error("privileged instruction {0} in problem state")]
    Privileged(&'static str),
    /// A trap whose TO-selected condition held.
    ///
    /// [PPC-Book1 p:62 s:3.3.10 Fixed-Point Trap Instructions] tw/td invoke the system trap handler when any TO-selected condition holds.
    #[error("trap (TO=0x{to:02x})")]
    Trap {
        /// The trap instruction's TO field.
        to: u8,
    },
}

impl ProgramCause {
    /// The one SRR1 cause bit this cause sets; bit 47 stays clear,
    /// because SRR0 addresses the instruction that caused it.
    fn srr1(self) -> u64 {
        match self {
            Self::FpEnabled => irq::SRR1_PROGRAM_FP_ENABLED,
            Self::Illegal(_) => irq::SRR1_PROGRAM_ILLEGAL,
            Self::Privileged(_) => irq::SRR1_PROGRAM_PRIVILEGED,
            Self::Trap { .. } => irq::SRR1_PROGRAM_TRAP,
        }
    }
}

impl PpuInterrupt {
    /// The data storage interrupt for a load or store that memory
    /// refused, or `None` for an error no load or store reports.
    pub fn data_storage(error: &cellgov_mem::MemError, store: bool) -> Option<Self> {
        let (ea, protection) = match error {
            cellgov_mem::MemError::Unmapped(ctx) => (ctx.addr, false),
            cellgov_mem::MemError::ReservedWrite { addr, .. } => (*addr, true),
            _ => return None,
        };
        Some(Self::DataStorage {
            ea,
            store,
            protection,
        })
    }

    /// The effective address of the interrupt vector.
    fn vector(&self) -> u64 {
        match self {
            Self::DataStorage { .. } => irq::VECTOR_DATA_STORAGE,
            Self::DataSegment { .. } => irq::VECTOR_DATA_SEGMENT,
            Self::InstructionStorage => irq::VECTOR_INSTRUCTION_STORAGE,
            Self::InstructionSegment => irq::VECTOR_INSTRUCTION_SEGMENT,
            Self::Alignment { .. } => irq::VECTOR_ALIGNMENT,
            Self::Program(_) => irq::VECTOR_PROGRAM,
            Self::FpUnavailable => irq::VECTOR_FP_UNAVAILABLE,
            Self::VxuUnavailable => irq::VECTOR_VXU_UNAVAILABLE,
            Self::SystemCall { .. } => irq::VECTOR_SYSTEM_CALL,
        }
    }

    /// The SRR1 cause bits the interrupt sets.
    fn srr1(&self) -> u64 {
        match self {
            // [PPC-Book3 p:68 s:5.5.5] No translation found sets SRR1 bit 33.
            Self::InstructionStorage => irq::SRR1_FETCH_NOT_FOUND,
            Self::Program(cause) => cause.srr1(),
            _ => 0,
        }
    }

    /// The DSISR the interrupt sets, for the interrupts that set it.
    ///
    /// [CBE-Handbook p:253 s:9.5.8] The PPE does not modify DSISR on an alignment interrupt.
    fn dsisr(&self) -> Option<u32> {
        let store_bit = |store: bool| if store { irq::DSISR_STORE } else { 0 };
        match *self {
            Self::DataStorage {
                store, protection, ..
            } => {
                let cause = if protection {
                    irq::DSISR_PROTECTION
                } else {
                    irq::DSISR_NOT_FOUND
                };
                Some(cause | store_bit(store))
            }
            Self::DataSegment { store, .. } => Some(irq::DSISR_SEGMENT | store_bit(store)),
            _ => None,
        }
    }

    /// The DAR the interrupt sets, for the interrupts that set it.
    fn dar(&self) -> Option<u64> {
        match *self {
            Self::DataStorage { ea, .. } | Self::DataSegment { ea, .. } => Some(ea),
            // [PPC-Book3 p:70 s:5.5.8] An alignment interrupt sets DAR to the access's EA.
            Self::Alignment { ea, .. } => Some(ea),
            _ => None,
        }
    }

    /// Whether the access the interrupt refused was a store, for the
    /// interrupts that refuse an access.
    fn store(&self) -> Option<bool> {
        match *self {
            Self::DataStorage { store, .. }
            | Self::DataSegment { store, .. }
            | Self::Alignment { store, .. } => Some(store),
            _ => None,
        }
    }

    /// The interrupt registers after the instruction at `pc` takes the
    /// interrupt.
    pub(crate) fn record(&self, pc: u64) -> InterruptRecord {
        // [PPC-Book3 p:12 s:2.3.1] A system call interrupt sets SRR0 to the instruction after `sc`.
        // Every other class sets SRR0 to the instruction that caused it.
        let srr0 = match self {
            Self::SystemCall { .. } => pc.wrapping_add(4),
            _ => pc,
        };
        InterruptRecord {
            vector: self.vector(),
            srr0,
            srr1: self.srr1(),
            dsisr: self.dsisr(),
            dar: self.dar(),
        }
    }

    fn class(&self) -> PpuFaultClass {
        match self {
            Self::DataStorage { .. } => PpuFaultClass::DataStorage,
            Self::DataSegment { .. } => PpuFaultClass::DataSegment,
            Self::InstructionStorage => PpuFaultClass::InstructionStorage,
            Self::InstructionSegment => PpuFaultClass::InstructionSegment,
            Self::Alignment { .. } => PpuFaultClass::Alignment,
            Self::Program(ProgramCause::FpEnabled) => PpuFaultClass::ProgramFpEnabled,
            Self::Program(ProgramCause::Illegal(_)) => PpuFaultClass::ProgramIllegal,
            Self::Program(ProgramCause::Privileged(_)) => PpuFaultClass::ProgramPrivileged,
            Self::Program(ProgramCause::Trap { .. }) => PpuFaultClass::ProgramTrap,
            Self::FpUnavailable => PpuFaultClass::FpUnavailable,
            Self::VxuUnavailable => PpuFaultClass::VxuUnavailable,
            Self::SystemCall { .. } => PpuFaultClass::SystemCall,
        }
    }

    /// The guest fault code: the class prefix and the class's detail.
    fn guest_code(&self) -> u32 {
        let dsisr_high = |i: &Self| i.dsisr().map_or(0, |d| d >> 16);
        match *self {
            Self::DataStorage { .. } => FAULT_DATA_STORAGE | dsisr_high(self),
            Self::DataSegment { .. } => FAULT_DATA_SEGMENT | dsisr_high(self),
            Self::InstructionStorage => FAULT_INSTRUCTION_STORAGE | (self.srr1() >> 16) as u32,
            Self::InstructionSegment => FAULT_INSTRUCTION_SEGMENT,
            Self::Alignment { .. } => FAULT_ALIGNMENT,
            Self::Program(cause) => {
                let to = match cause {
                    ProgramCause::Trap { to } => u32::from(to),
                    _ => 0,
                };
                FAULT_PROGRAM | ((cause.srr1() >> 8) as u32 & 0xFF00) | (to & 0xFF)
            }
            Self::FpUnavailable => FAULT_FP_UNAVAILABLE,
            Self::VxuUnavailable => FAULT_VXU_UNAVAILABLE,
            Self::SystemCall { lev } => FAULT_SYSTEM_CALL | u32::from(lev),
        }
    }
}

impl PpuFault {
    /// An illegal-instruction program interrupt for the instruction `name`.
    pub fn illegal(name: &'static str) -> Self {
        Self::Interrupt(PpuInterrupt::Program(ProgramCause::Illegal(name)))
    }

    /// The illegal-instruction program interrupt for a word that decodes
    /// to no instruction.
    pub fn unrecognized_word() -> Self {
        Self::illegal("unrecognized word")
    }

    /// A privileged-instruction program interrupt for the instruction `name`.
    pub fn privileged(name: &'static str) -> Self {
        Self::Interrupt(PpuInterrupt::Program(ProgramCause::Privileged(name)))
    }

    /// A trap program interrupt with the trap's TO field.
    pub fn trap(to: u8) -> Self {
        Self::Interrupt(PpuInterrupt::Program(ProgramCause::Trap { to }))
    }

    /// An alignment interrupt on the access at `ea`.
    pub fn alignment(ea: u64, store: bool) -> Self {
        Self::Interrupt(PpuInterrupt::Alignment { ea, store })
    }

    /// Fills the fault fields of `diag` for this fault at `pc`: the EA
    /// of the fault, the refused access and the interrupt registers. A
    /// CellGov stop fills none.
    pub fn annotate(&self, pc: u64, diag: &mut LocalDiagnostics) {
        let Some(interrupt) = self.interrupt() else {
            return;
        };
        diag.faulting_ea = interrupt.dar();
        diag.data_access = interrupt.store().map(|store| DataAccessFault {
            store,
            refused_by: None,
        });
        diag.interrupt = Some(interrupt.record(pc));
    }

    /// The PPE interrupt, or `None` for a CellGov stop.
    pub(crate) fn interrupt(&self) -> Option<&PpuInterrupt> {
        match self {
            Self::Interrupt(interrupt) => Some(interrupt),
            _ => None,
        }
    }

    /// The fault class.
    pub fn class(&self) -> PpuFaultClass {
        match self {
            Self::Interrupt(interrupt) => interrupt.class(),
            Self::UnimplementedInstruction { .. } => PpuFaultClass::Unimplemented,
            Self::DebugBreak => PpuFaultClass::DebugBreak,
            Self::Internal(_) => PpuFaultClass::Internal,
        }
    }

    /// Returns the guest fault code published by the execution unit.
    pub fn guest_code(&self) -> u32 {
        match self {
            Self::Interrupt(interrupt) => interrupt.guest_code(),
            Self::UnimplementedInstruction { kind, .. } => {
                FAULT_UNIMPLEMENTED_INSN | u32::from(kind_index(*kind))
            }
            Self::DebugBreak => FAULT_DEBUG_BREAK,
            Self::Internal(_) => FAULT_INTERNAL,
        }
    }
}
