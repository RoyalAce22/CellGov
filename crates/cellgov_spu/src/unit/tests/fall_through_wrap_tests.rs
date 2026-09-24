//! Sequential execution past the last local-store word.

use crate::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_LS_SIZE;
use cellgov_time::Budget;

/// `nop`, RR opcode 0x201 in the high 11 bits.
const NOP: u32 = 0x201 << 21;

// [SPU-ISA p:31 s:3] Every local-storage address is ANDed with the LSLR, so the word after 0x3FFFC is 0.
#[test]
fn a_non_branch_at_the_last_local_store_word_falls_through_to_word_0() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(7));
    let last = SPU_LS_SIZE - 4;
    unit.state_mut().ls[last..].copy_from_slice(&NOP.to_be_bytes());
    unit.state_mut().ls[..4].copy_from_slice(&NOP.to_be_bytes());
    unit.state_mut().pc = last as u32;

    let mem = GuestMemory::new(0x2000);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(2), &ctx, &mut effects);

    // Both nops retired: the one at the last word and the one at word 0.
    assert_eq!(result.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(result.fault, None);
    assert_eq!(unit.state().pc, 4);
}
