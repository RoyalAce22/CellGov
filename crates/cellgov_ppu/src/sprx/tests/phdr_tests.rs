//! The vaddr-to-file-offset map on header fields near `usize::MAX`.

use super::{v2f, SegEntry};

#[test]
fn a_file_offset_that_overflows_maps_to_nothing() {
    let seg_map = [SegEntry {
        vaddr: 0x1000,
        file_offset: usize::MAX - 2,
        size: 0x100,
    }];
    assert_eq!(v2f(&seg_map, 0x1000), Some(usize::MAX - 2));
    assert_eq!(v2f(&seg_map, 0x1010), None);
}

#[test]
fn a_segment_whose_end_overflows_still_maps_its_start() {
    let seg_map = [SegEntry {
        vaddr: usize::MAX - 0xf,
        file_offset: 0x40,
        size: 0x100,
    }];
    assert_eq!(v2f(&seg_map, usize::MAX - 0xf), Some(0x40));
    assert_eq!(v2f(&seg_map, usize::MAX), Some(0x4f));
    assert_eq!(v2f(&seg_map, 0), None);
}
