//! A fused `ld; ld` whose second load faults logs the first load's
//! provisional read once, as the two loads unfused do.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{
    ByteRange, GuestAddr, GuestMemory, PageSize, ProvisionalRead, Region, RegionAccess,
};
use cellgov_time::Budget;

/// The base of the reserved-zero region.
const RESERVED: u64 = 0x1_0000;
/// The size of the reserved-zero region; unmapped memory follows it.
const RESERVED_LEN: u64 = 0x1000;

/// `ld rT, d(r5)`.
fn ld_r5(rt: u32, d: u16) -> u32 {
    (58 << 26) | (rt << 21) | (5 << 16) | u32::from(d)
}

/// Runs `ld r6, 0(r5); ld r7, 8(r5)` with r5 on the last doubleword of
/// the reserved-zero region, fused or by plain decode, until the second
/// load faults.
fn run(fused: bool) -> GuestMemory {
    let bytes = [ld_r5(6, 0), ld_r5(7, 8)]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect::<Vec<_>>();
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "flat", PageSize::Page4K),
        Region::with_access(
            RESERVED,
            RESERVED_LEN as usize,
            "reserved",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap(),
        &bytes,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        let shadow = shadow::PredecodedShadow::build(0, &bytes);
        assert!(
            shadow.get(0).is_some_and(|insn| insn.is_super_pair()),
            "the two loads must fuse, or the fused run tests nothing"
        );
        unit.set_instruction_shadow(shadow);
    }
    unit.state_mut().set_gpr(5, RESERVED + RESERVED_LEN - 8);
    for _ in 0..4 {
        let r = unit.run_until_yield(
            Budget::new(8),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        if r.yield_reason == YieldReason::Fault {
            return mem;
        }
    }
    panic!("the second load never faulted");
}

#[test]
fn a_fused_pair_whose_second_load_faults_logs_the_first_read_once() {
    for fused in [false, true] {
        let mem = run(fused);
        assert_eq!(mem.provisional_read_count(), 1, "fused {fused}");
        assert_eq!(
            mem.drain_provisional_reads(),
            [ProvisionalRead {
                addr: RESERVED + RESERVED_LEN - 8,
                len: 8,
                hits: 1,
            }],
            "fused {fused}"
        );
    }
}
