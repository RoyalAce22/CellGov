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
