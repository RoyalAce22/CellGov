//! Region labelling and longest-readable-prefix probing over guest memory.

use super::*;
use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_time::Budget;

fn rt_with_layout() -> Runtime {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x4000_0000, "main", PageSize::Page64K),
        Region::new(0xD000_0000, 0x0001_0000, "stack", PageSize::Page4K),
    ])
    .unwrap();
    Runtime::new(mem, Budget::new(1), 100)
}

#[test]
fn region_label_at_names_stack_region() {
    let rt = rt_with_layout();
    assert_eq!(region_label_at(&rt, 0xD000_FFF0, 4), "stack");
}

#[test]
fn region_label_at_names_main_region() {
    let rt = rt_with_layout();
    assert_eq!(region_label_at(&rt, 0x0010_0000, 4), "main");
}

#[test]
fn region_label_at_unmapped_addr_is_not_misattributed() {
    let rt = rt_with_layout();
    assert_eq!(region_label_at(&rt, 0x8000_0000, 4), "<unmapped>");
}

#[test]
fn longest_readable_prefix_returns_none_on_zero_length() {
    let rt = rt_with_layout();
    assert!(longest_readable_prefix(rt.memory(), 0, 0).is_none());
}

#[test]
fn longest_readable_prefix_returns_none_for_entirely_unmapped_buffer() {
    let rt = rt_with_layout();
    assert!(longest_readable_prefix(rt.memory(), 0x8000_0000, 64).is_none());
}

#[test]
fn longest_readable_prefix_finds_region_boundary_exactly() {
    let rt = rt_with_layout();
    assert!(
        longest_readable_prefix(rt.memory(), 0x4000_0000, 1).is_none(),
        "precondition: nothing readable at main's end"
    );
    let buf = 0x4000_0000 - 16;
    let (n, bytes) = longest_readable_prefix(rt.memory(), buf, 64).expect("some prefix");
    assert_eq!(n, 16);
    assert_eq!(bytes.len(), 16);
}

#[test]
fn longest_readable_prefix_returns_full_len_when_fully_mapped() {
    let rt = rt_with_layout();
    let (n, bytes) = longest_readable_prefix(rt.memory(), 0x0010_0000, 64)
        .expect("fully readable should return Some");
    assert_eq!(n, 64);
    assert_eq!(bytes.len(), 64);
}

#[test]
fn longest_readable_prefix_single_byte_boundary() {
    let rt = rt_with_layout();
    let buf = 0x4000_0000 - 1;
    let (n, _bytes) = longest_readable_prefix(rt.memory(), buf, 2).expect("single-byte prefix");
    assert_eq!(n, 1);
}
