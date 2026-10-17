//! Heuristic scanning of guest data for sys_lwmutex handle slots, with sentinel and field validation.

use super::*;
use cellgov_ps3_abi::lv2::kernel_id::compose;
use cellgov_ps3_abi::lv2::process::{
    ALL_PROCESS_OBJECT_CLASS_IDS, SYS_EVENT_PORT_OBJECT, SYS_FS_FD_OBJECT, SYS_TIMER_OBJECT,
};

/// An lwmutex id in `slot`, as CellGov's kernel-id table mints one.
fn lwmutex_id(slot: u32) -> u32 {
    compose(SYS_LWMUTEX_OBJECT, 0, slot)
}

/// An lwcond id in `slot`, as CellGov's kernel-id table mints one.
fn lwcond_id(slot: u32) -> u32 {
    compose(SYS_LWCOND_OBJECT, 0, slot)
}

fn emit_lwmutex(buf: &mut Vec<u8>, attribute: u32, sleep_queue: u32) {
    buf.extend_from_slice(&LWMUTEX_FREE.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&attribute.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&sleep_queue.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes());
}

#[test]
fn finds_one_lwmutex_at_data_base() {
    let mut data = Vec::new();
    emit_lwmutex(&mut data, 0x22, lwmutex_id(13));
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert_eq!(ranges, vec![(0x860000 + 0x10)..(0x860000 + 0x14)]);
}

#[test]
fn finds_multiple_separated_by_padding() {
    // First struct at offset 0x00..0x20 (sleep_queue at 0x10).
    // 16 bytes of padding at 0x20..0x30.
    // Second struct at offset 0x30..0x50 (sleep_queue at 0x40).
    let mut data = Vec::new();
    emit_lwmutex(&mut data, 0x22, lwmutex_id(13));
    data.extend_from_slice(&[0u8; 16]);
    emit_lwmutex(&mut data, 0x21, lwmutex_id(14));
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert_eq!(ranges, vec![0x860010..0x860014, 0x860040..0x860044]);
}

#[test]
fn rejects_wrong_sentinel() {
    let mut data = Vec::new();
    // Write a valid struct minus the sentinel.
    data.extend_from_slice(&[0u8; 4]); // owner = 0 (not lwmutex_free)
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&0x22u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&lwmutex_id(13).to_be_bytes());
    data.extend_from_slice(&[0u8; 12]);
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert!(ranges.is_empty());
}

#[test]
fn rejects_invalid_attribute() {
    let mut data = Vec::new();
    emit_lwmutex(&mut data, 0xdeadbeef, lwmutex_id(13));
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert!(ranges.is_empty());
}

#[test]
fn rejects_nonzero_pad() {
    let mut data = Vec::new();
    data.extend_from_slice(&LWMUTEX_FREE.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&0x22u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&lwmutex_id(13).to_be_bytes());
    data.extend_from_slice(&0xCAFEBABEu32.to_be_bytes()); // pad != 0
    data.extend_from_slice(&[0u8; 8]);
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert!(ranges.is_empty());
}

#[test]
fn rejects_a_sleep_queue_that_is_no_lwmutex_id() {
    for sleep_queue in [13, lwcond_id(13), runner_id(SYS_LWMUTEX_OBJECT, 32, 1)] {
        let mut data = Vec::new();
        emit_lwmutex(&mut data, 0x22, sleep_queue);
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        assert!(ranges.is_empty(), "{sleep_queue:#x}");
    }
}

#[test]
fn empty_or_too_small_data_returns_empty() {
    let ranges = find_sys_lwmutex_handle_slots(&[], 0x860000);
    assert!(ranges.is_empty());
    let ranges = find_sys_lwmutex_handle_slots(&[0u8; 8], 0x860000);
    assert!(ranges.is_empty());
}

#[test]
fn accepts_all_eight_valid_attribute_combos() {
    for attr in [0x11, 0x12, 0x13, 0x14, 0x21, 0x22, 0x23, 0x24] {
        let mut data = Vec::new();
        emit_lwmutex(&mut data, attr, lwmutex_id(1));
        let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
        assert_eq!(ranges.len(), 1, "attr 0x{attr:x} rejected unexpectedly");
    }
}

fn emit_lwcond(buf: &mut Vec<u8>, lwmutex_ptr: u32, lwcond_queue: u32) {
    buf.extend_from_slice(&lwmutex_ptr.to_be_bytes());
    buf.extend_from_slice(&lwcond_queue.to_be_bytes());
}

#[test]
fn an_lwcond_bound_to_a_found_lwmutex_yields_its_queue_slot() {
    let base = 0x860000u64;
    let mut data = Vec::new();
    emit_lwmutex(&mut data, 0x22, lwmutex_id(13)); // lwmutex at base+0x00..0x20
    emit_lwcond(&mut data, 0x860000, lwcond_id(4)); // lwcond at base+0x20
    let lwmutex = find_sys_lwmutex_handle_slots(&data, base);
    assert_eq!(lwmutex, vec![0x860010..0x860014]);
    assert_eq!(
        find_sys_lwcond_handle_slots(&data, base, &lwmutex),
        vec![0x860024..0x860028]
    );
}

#[test]
fn an_lwcond_pointing_elsewhere_or_holding_no_kernel_id_is_skipped() {
    let base = 0x860000u64;
    let mut data = Vec::new();
    emit_lwmutex(&mut data, 0x22, lwmutex_id(13));
    emit_lwcond(&mut data, 0x860040, lwcond_id(4)); // no lwmutex at 0x860040
    emit_lwcond(&mut data, 0x860000, 0); // never created: queue id still zero
    emit_lwcond(&mut data, 0x860000, lwmutex_id(4)); // an id of another class
    let lwmutex = find_sys_lwmutex_handle_slots(&data, base);
    assert!(find_sys_lwcond_handle_slots(&data, base, &lwmutex).is_empty());
}

#[test]
fn an_lwcond_scan_without_lwmutexes_finds_nothing() {
    let mut data = Vec::new();
    emit_lwcond(&mut data, 0x860000, lwcond_id(0));
    assert!(find_sys_lwcond_handle_slots(&data, 0x860000, &[]).is_empty());
}

#[test]
#[should_panic(expected = "starts below its struct base")]
fn an_lwmutex_slot_that_no_struct_can_hold_is_refused() {
    let mut data = Vec::new();
    emit_lwcond(&mut data, 0, lwcond_id(0));
    let below_any_sleep_queue = (SLEEP_QUEUE_OFFSET as u64 - 4)..(SLEEP_QUEUE_OFFSET as u64);
    find_sys_lwcond_handle_slots(&data, 0, &[below_any_sleep_queue]);
}

/// An id of `class` at `index` in the comparison runner's per-class
/// window, spelled the way its dumps carry one.
fn runner_id(class: ProcessObjectClassId, index: u32, reuse: u32) -> u32 {
    (class << 24) | (index * 0x100) | reuse
}

#[test]
fn a_runner_id_is_recognised_by_class_and_bounded_by_index() {
    assert_eq!(
        runner_kernel_handle_kind(runner_id(SYS_MUTEX_OBJECT, 1, 0)),
        Some(KernelHandleKind::Mutex)
    );
    assert_eq!(
        runner_kernel_handle_kind(runner_id(SYS_COND_OBJECT, 2, 3)),
        Some(KernelHandleKind::Cond),
        "the bytes under the stride are the allocator's reuse counter"
    );
    assert_eq!(
        runner_kernel_handle_kind(runner_id(SYS_EVENT_FLAG_OBJECT, 0, 0)),
        Some(KernelHandleKind::EventFlag)
    );
    assert_eq!(
        runner_kernel_handle_kind(runner_id(SYS_MUTEX_OBJECT, 8191, 0)),
        Some(KernelHandleKind::Mutex)
    );
    assert_eq!(
        runner_kernel_handle_kind(runner_id(SYS_MUTEX_OBJECT, 8192, 0)),
        None,
        "past the per-class count is not an id"
    );
    assert_eq!(runner_kernel_handle_kind(0x0001_0000), None);
    assert_eq!(
        runner_kernel_handle_kind(0xD100_0001),
        None,
        "a word whose top byte is no sync class"
    );
}

#[test]
fn every_recognised_class_is_a_sync_primitive_the_host_counts() {
    // A word the recogniser accepts always names a class CellGov
    // itself models. The two the table leaves out have their own
    // test below.
    let sync_classes: Vec<ProcessObjectClassId> = ALL_PROCESS_OBJECT_CLASS_IDS
        .iter()
        .copied()
        .filter(|c| ![SYS_EVENT_PORT_OBJECT, SYS_TIMER_OBJECT, SYS_FS_FD_OBJECT].contains(c))
        .collect();
    for class in sync_classes {
        assert!(
            runner_kernel_handle_kind(runner_id(class, 0, 0)).is_some(),
            "class {class:#x} is in the ABI list and the recogniser drops it: \
             add it to RUNNER_ID_CLASSES, or to this test's exclusions if its \
             id window can collide with a guest address"
        );
    }
}

#[test]
fn a_kernel_handle_pair_needs_one_class_on_both_sides_in_either_order() {
    let cg = compose(SYS_MUTEX_OBJECT, 1, 7);
    assert_eq!(
        kernel_handle_pair(runner_id(SYS_MUTEX_OBJECT, 3, 0), cg),
        Some(KernelHandleKind::Mutex)
    );
    assert_eq!(
        kernel_handle_pair(
            compose(SYS_SEMAPHORE_OBJECT, 0, 2),
            runner_id(SYS_SEMAPHORE_OBJECT, 1, 4)
        ),
        Some(KernelHandleKind::Semaphore)
    );
    assert_eq!(
        kernel_handle_pair(runner_id(SYS_LWMUTEX_OBJECT, 2, 0), lwmutex_id(3)),
        Some(KernelHandleKind::LwMutex)
    );
    assert_eq!(
        kernel_handle_pair(
            runner_id(SYS_MUTEX_OBJECT, 1, 0),
            runner_id(SYS_MUTEX_OBJECT, 2, 0)
        ),
        Some(KernelHandleKind::Mutex),
        "the two runners' shapes overlap, so one class on both sides pairs"
    );
    assert_eq!(
        kernel_handle_pair(runner_id(SYS_LWMUTEX_OBJECT, 2, 0), cg),
        None,
        "two classes"
    );
    assert_eq!(
        kernel_handle_pair(
            runner_id(SYS_MUTEX_OBJECT, 1, 1),
            runner_id(SYS_MUTEX_OBJECT, 2, 1)
        ),
        None,
        "a nonzero low byte on both sides: neither is CellGov's"
    );
    assert_eq!(
        kernel_handle_pair(runner_id(SYS_MUTEX_OBJECT, 1, 0), 0),
        None,
        "never created on the CellGov side"
    );
    let iomap_pointer =
        u32::try_from(cellgov_ps3_abi::hw::address_space::PS3_RSX_IOMAP_BASE).unwrap() + 1;
    assert_eq!(
        kernel_handle_pair(runner_id(SYS_MUTEX_OBJECT, 1, 0), iomap_pointer),
        None,
        "an RSX iomap pointer is not a CellGov kernel id"
    );
    assert_eq!(
        kernel_handle_pair(0x0001_2340, 0x0001_2344),
        None,
        "two pointers"
    );
}

#[test]
fn a_kind_whose_id_window_lies_in_guest_memory_is_never_paired() {
    for (class, slot) in [(SYS_EVENT_PORT_OBJECT, 1), (SYS_TIMER_OBJECT, 2)] {
        let word = runner_id(class, slot, 0);
        let cg = compose(class, 0, slot + 1);
        assert_eq!(runner_kernel_handle_kind(word), None, "{word:#x}");
        assert_eq!(kernel_handle_pair(word, cg), None, "{word:#x}");
        assert_eq!(kernel_handle_pair(cg, word), None, "{word:#x}");
    }
}
