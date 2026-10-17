//! Child-thread stacks held to the `lv2_address_space` capture from the
//! reference console. The probe creates, starts and joins four threads
//! in turn, with stacks of 0x4000, 0x8000, 0x10000 and 0x40000 bytes;
//! each records its r1 and its own `sys_ppu_thread_get_stack_information`
//! answer.

use super::*;

const SIZES: [u64; 4] = [0x4000, 0x8000, 0x1_0000, 0x4_0000];

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/lv2_address_space/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// Child n's (r1, stack address, stack size) on the console.
fn console_child(w: &[u32], n: usize) -> (u64, u64, u64) {
    let at = 21 + 5 * n;
    assert_eq!(w[at], 0, "child {n}: create");
    assert_eq!(w[at + 2], 0, "child {n}: stack information");
    (
        u64::from(w[at + 1]),
        u64::from(w[at + 3]),
        u64::from(w[at + 4]),
    )
}

#[test]
fn a_child_stack_is_the_size_asked_for_and_starts_r1_one_minimum_frame_below_its_top() {
    let w = words();
    let mut alloc = ThreadStackAllocator::new();
    for (n, &size) in SIZES.iter().enumerate() {
        let (r1, base, console_size) = console_child(&w, n);
        let stack = alloc.allocate(size, 0x10).expect("a stack");
        assert_eq!(stack.size(), console_size, "child {n}: size");
        assert_eq!(
            stack.end() - stack.initial_sp(),
            base + console_size - r1,
            "child {n}: r1 below the top"
        );
    }
}

/// The console gives each of the four threads the same stack address,
/// 0x2000 above the end of the default 64 KiB primary stack: a joined
/// thread's stack is reused. CellGov bumps from its own child region
/// and reuses none.
#[test]
fn the_console_reuses_one_child_stack_slot_and_cellgov_bumps_from_its_own_region() {
    let w = words();
    let console: Vec<u64> = (0..SIZES.len()).map(|n| console_child(&w, n).1).collect();
    assert!(
        console.iter().all(|&base| base == console[0]),
        "the console: {console:x?}"
    );
    let primary_end = u64::from(w[19]) + u64::from(w[20]);
    assert_eq!(console[0], primary_end + 0x2000, "the console's slot");

    let mut alloc = ThreadStackAllocator::new();
    let cellgov: Vec<u64> = SIZES
        .iter()
        .map(|&size| alloc.allocate(size, 0x10).expect("a stack").base())
        .collect();
    assert_eq!(cellgov[0], ThreadStackAllocator::CHILD_STACK_BASE);
    assert_ne!(cellgov[0], console[0], "CellGov's region is its own");
    assert!(
        cellgov.windows(2).all(|p| p[0] < p[1]),
        "CellGov: {cellgov:x?}"
    );
}
