//! A fault report's region label for a range across regions that sit end
//! to end.

use super::*;
use cellgov_mem::{PageSize, Region};

const SIZE: usize = 0x1_0000;
const EDGE: u64 = SIZE as u64;

#[test]
fn a_range_across_two_adjacent_regions_names_both() {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, SIZE, "low", PageSize::Page64K),
        Region::new(EDGE, SIZE, "high", PageSize::Page64K),
    ])
    .unwrap();
    assert_eq!(region_label_at(&mem, EDGE - 32, 64), "low+high");
    assert_eq!(region_label_at(&mem, EDGE - 64, 64), "low");
}

#[test]
fn a_range_across_a_gap_is_unmapped() {
    let mem = GuestMemory::from_regions(vec![
        Region::new(0, SIZE, "low", PageSize::Page64K),
        Region::new(EDGE + 0x10, SIZE, "high", PageSize::Page64K),
    ])
    .unwrap();
    assert_eq!(region_label_at(&mem, EDGE - 32, 64), "<unmapped>");
}
