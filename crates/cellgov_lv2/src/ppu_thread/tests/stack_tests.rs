//! Child-thread stack placement tests -- non-overlap with live stacks,
//! reuse of collected ranges, the arena's ceiling, alignment, and
//! initial-SP placement.

use super::*;

/// Place a block against the stacks in `live`.
fn place(live: &[ThreadStack], size: u64, align: u64) -> Option<ThreadStack> {
    place_child_stack(live.iter().map(|s| (s.base, s.size)), size, align)
}

/// Place `count` blocks of `size` in turn, each one live for the next.
fn place_all(count: usize, size: u64) -> Vec<ThreadStack> {
    let mut live = Vec::new();
    for _ in 0..count {
        let s = place(&live, size, 0x10).expect("a block");
        live.push(s);
    }
    live
}

#[test]
fn three_live_children_get_non_overlapping_rising_blocks() {
    let live = place_all(3, 0x10_000);
    assert_eq!(live[0].base, CHILD_STACK_BASE);
    assert_eq!(live[1].base, live[0].end());
    assert_eq!(live[2].base, live[1].end());
    assert!(live[0].base > 0xD000_FFFF);
}

#[test]
fn placement_depends_only_on_the_live_set() {
    assert_eq!(place_all(4, 0x10_000), place_all(4, 0x10_000));
}

#[test]
fn a_collected_threads_range_is_placed_again() {
    let mut live = place_all(3, 0x10_000);
    let freed = live.remove(1);
    let next = place(&live, 0x10_000, 0x10).expect("a block");
    assert_eq!(next, freed, "the lowest free range is the collected one");
}

#[test]
fn a_block_too_big_for_a_freed_gap_goes_above_the_live_blocks() {
    let mut live = place_all(3, 0x10_000);
    live.remove(1);
    let next = place(&live, 0x20_000, 0x10).expect("a block");
    assert_eq!(next.base, live[1].end());
}

#[test]
fn sequential_create_and_join_reuses_one_slot() {
    // Each thread is collected before the next create, so the live set
    // is empty every time.
    let bases: Vec<u64> = [0x4000, 0x8000, 0x1_0000, 0x4_0000]
        .iter()
        .map(|&size| place(&[], size, 0x10).expect("a block").base)
        .collect();
    assert!(bases.iter().all(|&b| b == CHILD_STACK_BASE), "{bases:x?}");
}

#[test]
fn live_stacks_past_the_first_15_mib_are_placed_inside_the_arena() {
    // 20 live 1 MiB stacks reach past 0xD1000000, where the old 15 MiB
    // region ended; every block still lies inside the arena.
    let live = place_all(20, 0x10_0000);
    let top = live.last().expect("20 blocks");
    assert!(
        top.end() > 0xD100_0000,
        "top block ends at {:#x}",
        top.end()
    );
    assert!(live
        .iter()
        .all(|s| s.base >= CHILD_STACK_BASE && s.end() <= CHILD_STACK_END));
}

#[test]
fn the_arena_ends_at_the_end_of_the_stack_segment() {
    assert_eq!(CHILD_STACK_END, 0xE000_0000);
}

#[test]
fn a_block_that_does_not_fit_below_the_arena_end_is_refused() {
    let arena = CHILD_STACK_END - CHILD_STACK_BASE;
    let whole = place(&[], arena, 0x10).expect("the whole arena fits");
    assert_eq!(whole.end(), CHILD_STACK_END);
    assert!(place(&[], arena + 0x10, 0x10).is_none());
    assert!(
        place(&[whole], ABI_MIN_STACK_FRAME, 0x10).is_none(),
        "a full arena places nothing"
    );
}

#[test]
fn a_full_arena_places_again_once_a_thread_is_collected() {
    let arena = CHILD_STACK_END - CHILD_STACK_BASE;
    let half = arena / 2;
    let a = place(&[], half, 0x10).expect("first half");
    let b = place(&[a], half, 0x10).expect("second half");
    assert!(place(&[a, b], 0x4000, 0x10).is_none());
    assert_eq!(place(&[b], 0x4000, 0x10).expect("a's range").base, a.base);
}

#[test]
fn stacks_outside_the_arena_do_not_constrain_placement() {
    let primary = ThreadStack {
        base: 0xD000_1000,
        size: 0x10_0000,
    };
    let spawned_primary = ThreadStack { base: 0, size: 0 };
    let s = place(&[primary, spawned_primary], 0x4000, 0x10).expect("a block");
    assert_eq!(s.base, CHILD_STACK_BASE);
}

#[test]
fn placement_honors_alignment() {
    let pad = place(&[], 0x4321, 0x10).expect("a block");
    let s = place(&[pad], 0x1000, 0x1000).expect("a block");
    assert_eq!(s.base & 0xFFF, 0, "base not 4 KiB-aligned");
    assert!(s.base >= pad.end());
}

#[test]
fn minimum_alignment_is_16_bytes() {
    let pad = place(&[], 0x71, 0x10).expect("a block");
    let s = place(&[pad], 0x100, 0).expect("a block");
    assert_eq!(s.base & 0xF, 0);
}

#[test]
fn placement_rejects_zero_size() {
    assert!(place(&[], 0, 0x10).is_none());
}

#[test]
fn placement_rejects_size_below_save_area() {
    assert!(place(&[], 0x8, 0x10).is_none());
    assert!(place(&[], 0xF, 0x10).is_none());
    assert!(
        place(&[], ABI_MIN_STACK_FRAME - 1, 0x10).is_none(),
        "a block one byte short of a minimum frame cannot hold one",
    );
    assert!(place(&[], ABI_MIN_STACK_FRAME, 0x10).is_some());
}

#[test]
fn placement_returns_none_when_the_size_would_overflow() {
    assert!(place(&[], u64::MAX - 0x100, 0x10).is_none());
}

#[test]
fn thread_stack_initial_sp_leaves_a_whole_minimum_frame() {
    let s = ThreadStack {
        base: 0xD001_0000,
        size: 0x10_000,
    };
    assert_eq!(ABI_MIN_STACK_FRAME, 0x70);
    assert_eq!(
        s.initial_sp(),
        0xD002_0000 - 0x70,
        "16 bytes is the header's first two doublewords, not a frame: a          callee saving LR at 16(r1) would write above the block",
    );
    assert_eq!(s.end(), 0xD002_0000);
    assert_eq!(s.end() - s.initial_sp(), ABI_MIN_STACK_FRAME);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "would underflow")]
fn thread_stack_initial_sp_debug_asserts_on_tiny_size() {
    let s = ThreadStack {
        base: 0x1000,
        size: 0x8,
    };
    let _ = s.initial_sp();
}
