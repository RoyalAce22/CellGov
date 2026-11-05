//! The fault report gives each PPU fault class its own line: the PPE
//! interrupt with its SRR0, SRR1 cause bits, DSISR and DAR, or the
//! CellGov stop.

use super::format_fault;
use crate::step_loop::PcRing;
use cellgov_core::Runtime;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::exec::{PpuFault, PpuInterrupt, ProgramCause};
use cellgov_ppu::PpuFaultClass;
use cellgov_time::{Budget, InstructionCost};
use strum::VariantArray;

/// The PC every fault in this file stops at.
const PC: u64 = 0x10;

/// `mcrxr cr5`, the word at [`PC`].
const MCRXR: u32 = 0x7E80_0400;

/// A fault of `class`, and the report line it must produce.
fn case(class: PpuFaultClass) -> (PpuFault, &'static str) {
    use PpuFaultClass as C;
    let interrupt = |i: PpuInterrupt| PpuFault::Interrupt(i);
    match class {
        C::InstructionStorage => (
            interrupt(PpuInterrupt::InstructionStorage { no_execute: false }),
            "INSTRUCTION_STORAGE at SRR0=0x00000010 SRR1[33:36]=0b1000",
        ),
        C::InstructionSegment => (
            interrupt(PpuInterrupt::InstructionSegment),
            "INSTRUCTION_SEGMENT at SRR0=0x00000010",
        ),
        C::DataStorage => (
            interrupt(PpuInterrupt::DataStorage {
                ea: 0x2_0000,
                store: true,
                protection: false,
            }),
            "DATA_STORAGE store at SRR0=0x00000010 DAR=0x00020000 DSISR=0x42000000",
        ),
        C::DataSegment => (
            interrupt(PpuInterrupt::DataSegment {
                ea: 0x2_0000,
                store: false,
            }),
            "DATA_SEGMENT load at SRR0=0x00000010 DAR=0x00020000 DSISR=0x00200000",
        ),
        C::Alignment => (
            PpuFault::alignment(0x2_0003, false),
            "ALIGNMENT load at SRR0=0x00000010 DAR=0x00020003",
        ),
        C::ProgramFpEnabled => (
            interrupt(PpuInterrupt::Program(ProgramCause::FpEnabled)),
            "PROGRAM_INTERRUPT fp-enabled at SRR0=0x00000010 SRR1[43:47]=0b10000",
        ),
        C::ProgramIllegal => (
            PpuFault::illegal("mcrxr"),
            "PROGRAM_INTERRUPT illegal at SRR0=0x00000010 SRR1[43:47]=0b01000 (raw=0x7e800400)",
        ),
        C::ProgramPrivileged => (
            PpuFault::privileged("mfmsr"),
            "PROGRAM_INTERRUPT privileged at SRR0=0x00000010 SRR1[43:47]=0b00100 (raw=0x7e800400)",
        ),
        C::ProgramTrap => (
            PpuFault::trap(0x04),
            "PROGRAM_INTERRUPT trap TO=0x04 at SRR0=0x00000010 SRR1[43:47]=0b00010",
        ),
        C::FpUnavailable => (
            interrupt(PpuInterrupt::FpUnavailable),
            "FP_UNAVAILABLE at SRR0=0x00000010",
        ),
        C::VxuUnavailable => (
            interrupt(PpuInterrupt::VxuUnavailable),
            "VXU_UNAVAILABLE at SRR0=0x00000010",
        ),
        C::SystemCall => (
            interrupt(PpuInterrupt::SystemCall { lev: 1 }),
            "SYSTEM_CALL LEV=1 (hypervisor call) at PC=0x00000010 SRR0=0x00000014",
        ),
        C::Unimplemented => (
            PpuFault::UnimplementedInstruction {
                kind: cellgov_ppu::instruction::PpuInstructionKind::Vx,
                mnemonic: "vx",
            },
            "UNIMPLEMENTED_INSN Vx at PC=0x00000010 (raw=0x7e800400)",
        ),
        C::DebugBreak => (PpuFault::DebugBreak, "DEBUG_BREAK at PC=0x00000010"),
        C::Internal => (
            PpuFault::Internal(".consumed"),
            "INTERNAL_STOP at PC=0x00000010 (raw=0x7e800400)",
        ),
    }
}

fn render(fault: &PpuFault) -> String {
    let mut mem = GuestMemory::new(0x1000);
    let range = ByteRange::new(GuestAddr::new(PC), 4).unwrap();
    mem.apply_commit(range, &MCRXR.to_be_bytes()).unwrap();
    let rt = Runtime::new(mem, Budget::new(1), 100);
    let mut diag = LocalDiagnostics::with_pc(PC);
    fault.annotate(PC, &mut diag);
    let kind = FaultKind::Guest(fault.guest_code());
    let result = ExecutionStepResult {
        yield_reason: YieldReason::Fault,
        consumed_cost: InstructionCost::new(0),
        local_diagnostics: diag,
        fault: Some(kind),
        syscall_args: None,
    };
    format_fault(&rt, UnitId::new(0), &result, &kind, 7, &PcRing::new(), &[])
}

#[test]
fn every_fault_class_has_its_own_report_line() {
    for &class in PpuFaultClass::VARIANTS {
        let (fault, want) = case(class);
        assert_eq!(fault.class(), class, "{fault:?}");
        let out = render(&fault);
        let first = out.lines().next().unwrap_or_default();
        assert_eq!(first, format!("FAULT at step 7: {want}"), "{class:?}");
    }
}
