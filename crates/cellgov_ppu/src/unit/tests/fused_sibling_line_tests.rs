//! A fused pair beside a runnable PPE sibling ends its batch where the
//! unfused pair ends it. That holds also when the first half reaches a grid
//! line of the thread's own clock.

use super::batch::SIBLING_LEAD_CYCLES;
use crate::instruction::{FusedPair, PpuInstruction};
use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks};

/// `lis r3, 0x1234`.
const LIS_R3: u32 = (15 << 26) | (3 << 21) | 0x1234;
/// `ori r3, r3, 0x00F0`.
const ORI_R3: u32 = (24 << 26) | (3 << 21) | (3 << 16) | 0x00F0;

/// The instructions a batch may retire.
const BUDGET: u64 = 256;

/// `BUDGET / 2` copies of `lis r3; ori r3, r3` from address 0.
fn code() -> Vec<u8> {
    (0..BUDGET / 2)
        .flat_map(|_| [LIS_R3, ORI_R3])
        .flat_map(u32::to_be_bytes)
        .collect()
}

/// One batch of [`code`] from tick `start` beside one runnable sibling: the
/// instructions it retired, the pc it stopped at and the cycles it took.
/// With `fused`, the unit runs from a fused shadow.
fn batch(code: &[u8], start: u64, fused: bool) -> (u64, u64, u64) {
    let mut mem = GuestMemory::new(0x1000);
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), code.len() as u64).expect("range"),
        code,
    )
    .expect("mapped");
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        let shadow = shadow::PredecodedShadow::build(0, code);
        assert!(
            matches!(
                shadow.get(0),
                Some(PpuInstruction::Fused(FusedPair::LisOri { .. }))
            ),
            "precondition: lis fuses with ori, got {:?}",
            shadow.get(0)
        );
        unit.set_instruction_shadow(shadow);
    }
    let ctx = ExecutionContext::new(&mem)
        .with_current_tick(GuestTicks::new(start))
        .with_ppe_siblings(1);
    let result = unit.run_until_yield(Budget::new(BUDGET), &ctx, &mut Vec::new());
    (
        unit.retired_instructions(),
        unit.state().pc,
        result.consumed_cost.raw(),
    )
}

#[test]
fn a_fused_pair_beside_a_sibling_ends_its_batch_where_the_unfused_pair_does() {
    let code = code();
    let mut ends_after_lis = 0;
    for into in 0..SIBLING_LEAD_CYCLES {
        let start = 3 * SIBLING_LEAD_CYCLES + into;
        let unfused = batch(&code, start, false);
        if unfused.0 % 2 == 1 {
            ends_after_lis += 1;
        }
        assert_eq!(batch(&code, start, true), unfused, "start tick {start}");
    }
    assert!(
        ends_after_lis > 0,
        "no start tick puts a grid line at a lis, so no case splits a pair"
    );
}
