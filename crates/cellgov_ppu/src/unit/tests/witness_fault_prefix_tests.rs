//! The unit runs a `dcbz`, `stwcx.` or `stdcx.` whose store faults
//! behind a retired prefix twice: once at the end of the prefix's batch,
//! and once alone. Neither run counts it in its witness counter.

use crate::*;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_sync::ReservedLine;
use cellgov_time::Budget;

const UNMAPPED_EA: u64 = 0xFFFF_0000;

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `dcbz 0, r5`.
const DCBZ_R5: u32 = (31 << 26) | (5 << 11) | (1014 << 1);
/// `stwcx. r6, 0, r5`.
const STWCX_R5: u32 = (31 << 26) | (6 << 21) | (5 << 11) | (150 << 1) | 1;
/// `stdcx. r6, 0, r5`.
const STDCX_R5: u32 = (31 << 26) | (6 << 21) | (5 << 11) | (214 << 1) | 1;

/// The `(dcbz, stwcx, stdcx)` witness counters.
fn counters(unit: &PpuExecutionUnit) -> (u64, u64, u64) {
    let s = unit.state();
    (s.dcbz_executed, s.stwcx_executed, s.stdcx_executed)
}

/// Runs `addi r3` and then `store` in batches until a batch faults.
///
/// r5 holds `UNMAPPED_EA`, and a reservation covers its line. Returns
/// the unit and the counters after each batch.
fn run_to_store_fault(store: u32) -> (PpuExecutionUnit, Vec<(u64, u64, u64)>) {
    let mut mem = GuestMemory::new(256);
    for (i, raw) in [ADDI_R3, store].into_iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, UNMAPPED_EA);
    unit.state_mut()
        .set_reservation(Some(ReservedLine::containing(UNMAPPED_EA)));
    let mut seen = Vec::new();
    for _ in 0..8 {
        let ctx = ExecutionContext::new(&mem);
        let mut effects = Vec::new();
        let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
        seen.push(counters(&unit));
        if r.yield_reason == YieldReason::Fault {
            assert_eq!(r.fault, Some(FaultKind::Guest(FAULT_INVALID_ADDRESS)));
            assert_eq!(r.local_diagnostics.pc, Some(4), "the store faults");
            return (unit, seen);
        }
        assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    }
    panic!("no fault within 8 batches");
}

#[test]
fn a_store_that_faults_behind_a_retired_prefix_is_never_counted() {
    for (name, store) in [
        ("dcbz", DCBZ_R5),
        ("stwcx.", STWCX_R5),
        ("stdcx.", STDCX_R5),
    ] {
        let (unit, seen) = run_to_store_fault(store);
        assert_eq!(seen.len(), 2, "{name}: the prefix batch, then the fault");
        assert_eq!(unit.state().gpr[3], 1, "{name}: the prefix retired");
        assert_eq!(unit.retired_instructions(), 1, "{name}");
        assert_eq!(seen, [(0, 0, 0), (0, 0, 0)], "{name}");
    }
}
