//! A batch runs on past an access to the reserved bytes between two
//! windows, and ends after an access to either window.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region, RegionAccess};
use cellgov_time::Budget;

const RESERVED: u64 = 0x1_0000;
/// The first window, with reserved bytes below it.
const FIRST: u64 = 0x2_0000;
/// The second window, with reserved bytes between it and `FIRST`.
const SECOND: u64 = 0x5_0000;
const WINDOW_BYTES: usize = 0x1_0000;

/// `addi rT, rT, 1`.
fn addi_inc(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

/// `lwz r6, 0(r5)`.
const LWZ_R6_R5: u32 = (32 << 26) | (6 << 21) | (5 << 16);
/// `stw r6, 0(r5)`.
const STW_R6_R5: u32 = (36 << 26) | (6 << 21) | (5 << 16);

/// Code at address 0, then a reserved region holding two windows that do
/// not touch.
fn memory(access: u32) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x1000, "main", PageSize::Page64K),
        Region::with_access(
            RESERVED,
            0x6_0000,
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
    mem.map_window(FIRST, WINDOW_BYTES, "first").unwrap();
    mem.map_window(SECOND, WINDOW_BYTES, "second").unwrap();
    mem
}

/// The instructions a batch of four retires with `access` at `ea`.
fn retired(access: u32, ea: u64) -> u64 {
    let mem = memory(access);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, ea);
    unit.state_mut().set_gpr(6, 0x1234_5678);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(4), &ctx, &mut effects);
    unit.retired_instructions()
}

#[test]
fn a_load_between_two_windows_runs_the_block_on() {
    // A store there faults: the reserved bytes refuse writes.
    let gap = FIRST + WINDOW_BYTES as u64 + 0x10;
    assert!(gap < SECOND);
    assert_eq!(retired(LWZ_R6_R5, gap), 4);
}

#[test]
fn a_load_or_store_in_either_window_ends_the_block_after_it() {
    for ea in [FIRST + 0x10, SECOND + 0x10] {
        for access in [LWZ_R6_R5, STW_R6_R5] {
            assert_eq!(retired(access, ea), 2, "access 0x{access:08x} at 0x{ea:x}");
        }
    }
}

#[test]
fn a_load_at_each_edge_of_the_gap_reaches_only_the_window_side() {
    let first_end = FIRST + WINDOW_BYTES as u64;
    // The last word of each window ends the block; the word just past
    // the first window and the word just below the second run it on.
    for (ea, want) in [
        (first_end - 4, 2),
        (first_end, 4),
        (SECOND - 4, 4),
        (SECOND, 2),
        (SECOND + WINDOW_BYTES as u64 - 4, 2),
    ] {
        assert_eq!(retired(LWZ_R6_R5, ea), want, "load at 0x{ea:x}");
    }
}
