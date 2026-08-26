//! Heuristic scanning of guest data for sys_lwmutex handle slots, with sentinel and field validation.

use super::*;

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
    emit_lwmutex(&mut data, 0x22, 13);
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert_eq!(ranges, vec![(0x860000 + 0x10)..(0x860000 + 0x14)]);
}

#[test]
fn finds_multiple_separated_by_padding() {
    // First struct at offset 0x00..0x20 (sleep_queue at 0x10).
    // 16 bytes of padding at 0x20..0x30.
    // Second struct at offset 0x30..0x50 (sleep_queue at 0x40).
    let mut data = Vec::new();
    emit_lwmutex(&mut data, 0x22, 13);
    data.extend_from_slice(&[0u8; 16]);
    emit_lwmutex(&mut data, 0x21, 14);
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
    data.extend_from_slice(&13u32.to_be_bytes());
    data.extend_from_slice(&[0u8; 12]);
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert!(ranges.is_empty());
}

#[test]
fn rejects_invalid_attribute() {
    let mut data = Vec::new();
    emit_lwmutex(&mut data, 0xdeadbeef, 13);
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
    data.extend_from_slice(&13u32.to_be_bytes());
    data.extend_from_slice(&0xCAFEBABEu32.to_be_bytes()); // pad != 0
    data.extend_from_slice(&[0u8; 8]);
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    assert!(ranges.is_empty());
}

#[test]
fn rejects_large_sleep_queue_value() {
    let mut data = Vec::new();
    // sleep_queue = 0x95002000 (RPCS3-style id, larger than CG's plausible cap).
    emit_lwmutex(&mut data, 0x22, 0x95002000);
    let ranges = find_sys_lwmutex_handle_slots(&data, 0x860000);
    // CG's snapshot should never carry an id this large.
    assert!(ranges.is_empty());
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
        emit_lwmutex(&mut data, attr, 1);
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
    emit_lwmutex(&mut data, 0x22, 13); // lwmutex at base+0x00..0x20
    emit_lwcond(&mut data, 0x860000, FIRST_KERNEL_ID + 4); // lwcond at base+0x20
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
    emit_lwmutex(&mut data, 0x22, 13);
    emit_lwcond(&mut data, 0x860040, FIRST_KERNEL_ID + 4); // no lwmutex at 0x860040
    emit_lwcond(&mut data, 0x860000, 0); // never created: queue id still zero
    emit_lwcond(&mut data, 0x860000, 0x9700_0100); // an RPCS3-shaped id has no place in CG's snapshot
    let lwmutex = find_sys_lwmutex_handle_slots(&data, base);
    assert!(find_sys_lwcond_handle_slots(&data, base, &lwmutex).is_empty());
}

#[test]
fn an_lwcond_scan_without_lwmutexes_finds_nothing() {
    let mut data = Vec::new();
    emit_lwcond(&mut data, 0x860000, FIRST_KERNEL_ID);
    assert!(find_sys_lwcond_handle_slots(&data, 0x860000, &[]).is_empty());
}

#[test]
#[should_panic(expected = "starts below its struct base")]
fn an_lwmutex_slot_that_no_struct_can_hold_is_refused() {
    let mut data = Vec::new();
    emit_lwcond(&mut data, 0, FIRST_KERNEL_ID);
    let below_any_sleep_queue = (SLEEP_QUEUE_OFFSET as u64 - 4)..(SLEEP_QUEUE_OFFSET as u64);
    find_sys_lwcond_handle_slots(&data, 0, &[below_any_sleep_queue]);
}

#[test]
fn rpcs3_ids_are_recognised_by_kind_and_bounded_by_index() {
    assert_eq!(
        rpcs3_kernel_handle_kind(0x8500_0100),
        Some(KernelHandleKind::Mutex)
    );
    assert_eq!(
        rpcs3_kernel_handle_kind(0x8600_0203),
        Some(KernelHandleKind::Cond),
        "the low byte is the id manager's reuse counter"
    );
    assert_eq!(
        rpcs3_kernel_handle_kind(0x9800_0000),
        Some(KernelHandleKind::EventFlag)
    );
    assert_eq!(
        rpcs3_kernel_handle_kind(0x8500_0000 + 8191 * 0x100),
        Some(KernelHandleKind::Mutex)
    );
    assert_eq!(
        rpcs3_kernel_handle_kind(0x8500_0000 + 8192 * 0x100),
        None,
        "past id_count is not an id"
    );
    assert_eq!(rpcs3_kernel_handle_kind(0x0001_0000), None);
    assert_eq!(rpcs3_kernel_handle_kind(FIRST_KERNEL_ID), None);
}

#[test]
fn a_kernel_handle_pair_needs_one_shape_per_side_in_either_order() {
    let cg = FIRST_KERNEL_ID + 7;
    assert_eq!(
        kernel_handle_pair(0x8500_0300, cg),
        Some(KernelHandleKind::Mutex)
    );
    assert_eq!(
        kernel_handle_pair(cg, 0x9600_0100),
        Some(KernelHandleKind::Semaphore)
    );
    assert_eq!(
        kernel_handle_pair(0x9500_0200, 3),
        Some(KernelHandleKind::LwMutex),
        "lwmutex ids count from 1 on the CellGov side"
    );
    assert_eq!(kernel_handle_pair(0x9500_0200, cg), None);
    assert_eq!(kernel_handle_pair(cg, cg + 1), None, "two CellGov ids");
    assert_eq!(
        kernel_handle_pair(0x8500_0100, 0x8500_0200),
        None,
        "two RPCS3 ids"
    );
    assert_eq!(
        kernel_handle_pair(0x8500_0100, 0),
        None,
        "never created on the CellGov side"
    );
    assert_eq!(
        kernel_handle_pair(0x0001_2340, 0x0001_2344),
        None,
        "two pointers"
    );
}

#[test]
fn a_kind_whose_id_window_lies_in_guest_memory_is_never_paired() {
    let cg = FIRST_KERNEL_ID + 3;
    for word in [0x0e00_0100, 0x1100_0200] {
        assert_eq!(rpcs3_kernel_handle_kind(word), None, "{word:#x}");
        assert_eq!(kernel_handle_pair(word, cg), None, "{word:#x}");
        assert_eq!(kernel_handle_pair(cg, word), None, "{word:#x}");
    }
}
