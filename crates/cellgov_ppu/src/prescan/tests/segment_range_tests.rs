//! Tests for the file range a PT_LOAD segment covers.

use super::*;

#[test]
fn segment_inside_the_image_covers_offset_to_offset_plus_filesz() {
    assert_eq!(segment_file_range(0x40, 0x20, 0x100), Some((0x40, 0x60)));
}

#[test]
fn segment_running_past_the_image_is_clamped_to_its_end() {
    assert_eq!(segment_file_range(0x40, 0x1000, 0x100), Some((0x40, 0x100)));
}

#[test]
fn segment_starting_at_the_image_end_covers_nothing() {
    assert_eq!(segment_file_range(0x100, 0x20, 0x100), None);
}
