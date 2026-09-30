//! The unit records each retired barrier by address and kind. A barrier
//! changes nothing else a step produces.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{BarrierKind, ExecutionContext, ExecutionUnit, RetiredBarrier, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

const STW_R3_128: u32 = (36 << 26) | (3 << 21) | 128;
const LWZ_R4_128: u32 = (32 << 26) | (4 << 21) | 128;
const NOP: u32 = 0x6000_0000;
const SYNC: u32 = 0x7c00_04ac;
const LWSYNC: u32 = 0x7c20_04ac;
const EIEIO: u32 = 0x7c00_06ac;
const ISYNC: u32 = 0x4c00_012c;

/// Runs `program` from address 0 with r3 = 0x1234 and r5 at an
/// address outside guest memory.
fn run(program: &[u32], per_step: bool) -> (PpuExecutionUnit, Vec<Effect>, Vec<RetiredBarrier>) {
    let mut mem = GuestMemory::new(256);
    for (i, &word) in program.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(3, 0x1234);
    unit.state_mut().set_gpr(5, 0xFFFF_0000);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(per_step);
    let mut effects = Vec::new();
    let budget = Budget::new(program.len() as u64);
    let result = unit.run_until_yield(budget, &ctx, &mut effects);
    assert_ne!(result.yield_reason, YieldReason::Finished);
    let barriers = unit.drain_barriers();
    (unit, effects, barriers)
}

#[test]
fn each_barrier_is_recorded_in_program_order_and_changes_nothing_else() {
    let with = [STW_R3_128, LWSYNC, LWZ_R4_128, EIEIO, ISYNC, SYNC];
    let without = [STW_R3_128, NOP, LWZ_R4_128, NOP, NOP, NOP];
    let (unit, effects, barriers) = run(&with, true);
    let (plain, plain_effects, none) = run(&without, true);
    assert_eq!(
        barriers,
        [
            RetiredBarrier {
                pc: 4,
                kind: BarrierKind::Lwsync
            },
            RetiredBarrier {
                pc: 12,
                kind: BarrierKind::Eieio
            },
            RetiredBarrier {
                pc: 16,
                kind: BarrierKind::Isync
            },
            RetiredBarrier {
                pc: 20,
                kind: BarrierKind::Sync
            },
        ]
    );
    assert!(none.is_empty());
    assert_eq!(unit.state().gpr[4], 0x1234, "the load still sees the store");
    assert_eq!(effects, plain_effects);
    assert_eq!(unit.state().state_hash(), plain.state().state_hash());
}

#[test]
fn a_step_without_per_step_tracing_records_no_barrier() {
    let (_, _, barriers) = run(&[LWSYNC, SYNC], false);
    assert!(barriers.is_empty(), "{barriers:?}");
}

#[test]
fn a_batch_that_faults_drops_the_barriers_it_retired() {
    let lwz_r6_r5 = (32 << 26) | (6 << 21) | (5 << 16);
    let (unit, _, barriers) = run(&[LWSYNC, lwz_r6_r5], true);
    assert_eq!(unit.status(), cellgov_exec::UnitStatus::Faulted);
    assert!(barriers.is_empty(), "{barriers:?}");
}
