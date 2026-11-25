//! A real-mode `ld` on a thread with the initial partition: an EA below
//! 256 GB reaches its real address, and an EA of 256 GB takes the data
//! storage interrupt with the protection cause.
//!
//! [CBE-Handbook p:336 s:11.2] Table 11-1: the largest RMLS limit is 256 GB, and an access past the limit takes the ISI or DSI with SRR1\[36\] = 1.
//! [PPC-Book3 p:67 s:5.5.3] A Data Storage interrupt sets SRR0 to the instruction's EA, DSISR bit 4 for a storage protection refusal, and DAR to the access's EA.

use crate::fault_codes::FAULT_CHECKSTOP;
use crate::supervisor::SupervisorState;
use crate::*;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_ME, MSR_SF};
use cellgov_time::Budget;

/// The real address of the code.
const CODE: u64 = 0x1000;
/// 256 GB: the first EA (bits 2:63) the initial partition refuses.
const LIMIT: u64 = 0x0000_0040_0000_0000;

/// `ld r3,0(r4)`.
const LD_R3_0_R4: u32 = (58 << 26) | (3 << 21) | (4 << 16);

/// Runs `ld r3,0(r4)` for one instruction on a 64-bit real-mode thread
/// with the initial partition and r4 = `ea`.
fn run_ld(ea: u64) -> (ExecutionStepResult, Vec<Effect>) {
    let mut mem = GuestMemory::new(0x1_0000);
    let range = ByteRange::new(GuestAddr::new(CODE), 4).unwrap();
    mem.apply_commit(range, &LD_R3_0_R4.to_be_bytes()).unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut()
        .set_supervisor(Some(SupervisorState::new(MSR_SF | MSR_ME)));
    unit.state_mut().pc = CODE;
    unit.state_mut().set_gpr(4, ea);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(1), &ExecutionContext::new(&mem), &mut effects);
    (r, effects)
}

#[test]
fn a_real_mode_ld_below_256_gb_reads_its_own_real_address() {
    let ea = 0x8000_0000_0000_8000;
    let (r, effects) = run_ld(ea);
    assert_eq!(r.local_diagnostics.interrupt, None);
    let reads: Vec<(u64, u64)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some((range.start().raw(), range.length())),
            _ => None,
        })
        .filter(|&(at, _)| at != CODE)
        .collect();
    assert_eq!(reads, vec![(0x8000, 8)]);
}

/// The translation passes, so the cacheable load reaches a real address
/// that no storage holds. CellGov stops that access as a checkstop, with
/// no interrupt.
#[test]
fn a_real_mode_ld_of_the_last_doubleword_below_256_gb_passes_translation_and_stops() {
    let (r, _) = run_ld(LIMIT - 8);
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(r.fault, Some(FaultKind::Guest(FAULT_CHECKSTOP)));
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_eq!(r.local_diagnostics.pc, Some(CODE));
}

#[test]
fn a_real_mode_ld_at_256_gb_takes_the_data_storage_interrupt_with_the_protection_cause() {
    let (r, _) = run_ld(LIMIT);
    let record: InterruptRecord = r.local_diagnostics.interrupt.expect("an interrupt");
    assert_eq!(record.vector, irq::VECTOR_DATA_STORAGE);
    assert_eq!(record.srr0, CODE);
    assert_eq!(record.dsisr, Some(irq::DSISR_PROTECTION));
    assert_eq!(record.dar, Some(LIMIT));
}
