//! A guest fault with no named arm renders the whole address the
//! diagnostics carry, or none when they carry none.

use super::format_fault;
use crate::step_loop::PcRing;
use cellgov_core::Runtime;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

/// A class no PPU arm names. The SPU's local-store class has this
/// value; its constant is private to `cellgov_spu`.
const UNNAMED_CLASS: u32 = 0x0002_0000;

fn render(diag: LocalDiagnostics, code: u32) -> String {
    let rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 100);
    let fault = FaultKind::Guest(code);
    let result = ExecutionStepResult {
        yield_reason: YieldReason::Fault,
        consumed_cost: InstructionCost::new(0),
        local_diagnostics: diag,
        fault: Some(fault),
        syscall_args: None,
    };
    format_fault(&rt, UnitId::new(0), &result, &fault, 7, &PcRing::new(), &[])
}

#[test]
fn an_unnamed_fault_renders_the_address_its_detail_half_truncated() {
    let out = render(
        LocalDiagnostics::with_pc_ea(0x4, 0x3_FF00),
        UNNAMED_CLASS | 0xFF00,
    );
    let want = "Guest(0x0002ff00) at PC=0x00000004 (ea=0x0003ff00)";
    assert!(out.contains(want), "got {out}");
}

#[test]
fn an_unnamed_fault_renders_a_64_bit_address_whole() {
    let out = render(
        LocalDiagnostics::with_pc_ea(0x4, 0x1_0000_0000),
        UNNAMED_CLASS,
    );
    assert!(out.contains("(ea=0x100000000)"), "got {out}");
}

#[test]
fn an_unnamed_fault_without_an_address_names_none() {
    let out = render(LocalDiagnostics::with_pc(0x3_FFFC), UNNAMED_CLASS);
    assert!(
        out.contains("Guest(0x00020000) at PC=0x0003fffc"),
        "got {out}"
    );
    assert!(!out.contains("(ea="), "got {out}");
}

#[test]
fn an_alignment_interrupt_names_itself_and_its_address() {
    let out = render(
        LocalDiagnostics::with_pc_ea(0x10, 0x2_0003),
        cellgov_ppu::FAULT_ALIGNMENT_INTERRUPT,
    );
    assert!(
        out.contains("ALIGNMENT_INTERRUPT at PC=0x00000010 (ea=0x00020003)"),
        "got {out}"
    );
}
