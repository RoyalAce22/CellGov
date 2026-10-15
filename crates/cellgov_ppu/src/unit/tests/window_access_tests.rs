//! A batch ends at the instruction that reaches a mapped window, so
//! the runtime can update the window before the next instruction runs.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_time::Budget;

const RESERVED: u64 = 0x1_0000;
const WINDOW: u64 = 0x2_0000;

/// `addi rT, rT, 1`.
fn addi_inc(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

/// `lwz r6, 0(r5)`.
const LWZ_R6_R5: u32 = (32 << 26) | (6 << 21) | (5 << 16);
/// `stw r6, 0(r5)`.
const STW_R6_R5: u32 = (36 << 26) | (6 << 21) | (5 << 16);

/// Code at address 0, a reserved region above it, and a window carved
/// from that region when `window` is set.
fn memory(access: u32, window: bool) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            RESERVED,
            0x4_0000,
            "reserved",
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let code = [addi_inc(3), access, addi_inc(4), addi_inc(4)];
    for (i, word) in code.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    if window {
        mem.map_window(WINDOW, 0x1_0000, "window").unwrap();
    }
    mem
}

fn run(mem: &GuestMemory, ea: u64) -> (PpuExecutionUnit, ExecutionStepResult, Vec<Effect>) {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, ea);
    unit.state_mut().set_gpr(6, 0x1234_5678);
    let ctx = ExecutionContext::new(mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(4), &ctx, &mut effects);
    (unit, result, effects)
}

#[test]
fn a_load_from_a_window_ends_the_block_after_it() {
    let mem = memory(LWZ_R6_R5, true);
    let (unit, result, effects) = run(&mem, WINDOW + 0x10);
    assert_eq!(result.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(result.consumed_cost.raw(), 2);
    assert_eq!(unit.state().pc, 8, "the next instruction has not run");
    assert_eq!(unit.state().gpr[4], 0);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SharedReadIntent { range, .. } if range.start().raw() == WINDOW + 0x10
    )));
}

#[test]
fn a_store_to_a_window_ends_the_block_after_it() {
    let mem = memory(STW_R6_R5, true);
    let (unit, result, effects) = run(&mem, WINDOW + 0x10);
    assert_eq!(result.consumed_cost.raw(), 2);
    assert_eq!(unit.state().pc, 8);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::SharedWriteIntent { range, .. } if range.start().raw() == WINDOW + 0x10
    )));
}

#[test]
fn an_access_outside_every_window_runs_the_block_on() {
    for (window, ea) in [(true, RESERVED + 0x10), (false, WINDOW + 0x10)] {
        let mem = memory(LWZ_R6_R5, window);
        let (unit, result, _) = run(&mem, ea);
        assert_eq!(
            result.consumed_cost.raw(),
            4,
            "window {window}, ea 0x{ea:x}"
        );
        assert_eq!(unit.state().gpr[4], 2);
    }
}
