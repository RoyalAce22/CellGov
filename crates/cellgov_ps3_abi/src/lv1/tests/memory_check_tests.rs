//! The allocate_memory check results hold the Linux-named status in the
//! low word and the failing argument's mark in the high word.

use super::{
    allocate_memory_checks_read, ALLOCATE_MEMORY_FLAGS_R3, ALLOCATE_MEMORY_PAGE_SIZE_EXP_MAX,
    ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN, ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3, ALLOCATE_MEMORY_R5_R3,
    ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3, ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3, ALLOCATE_MEMORY_WORKERS,
};
use crate::lv1::status::{zero_extended, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};

/// The r3 of a check whose high word holds `mark`.
fn marked(mark: u64, status: i64) -> u64 {
    (mark << 32) | zero_extended(status)
}

#[test]
fn each_check_result_marks_its_arguments_over_its_status() {
    let illegal = LV1_ILLEGAL_PARAMETER_VALUE;
    assert_eq!(ALLOCATE_MEMORY_SIZE_TOO_LARGE_R3, marked(0b0001, illegal));
    assert_eq!(ALLOCATE_MEMORY_PAGE_SIZE_EXP_R3, marked(0b0010, illegal));
    assert_eq!(ALLOCATE_MEMORY_SIZE_ALIGNMENT_R3, marked(0b0011, illegal));
    assert_eq!(ALLOCATE_MEMORY_R5_R3, marked(0b0100, LV1_NO_ENTRY));
    assert_eq!(ALLOCATE_MEMORY_FLAGS_R3, marked(0b1000, illegal));
}

#[test]
fn the_page_size_exponent_range_is_twelve_to_sixty_one() {
    assert_eq!(ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN, 12);
    assert_eq!(
        ALLOCATE_MEMORY_PAGE_SIZE_EXP_MAX - ALLOCATE_MEMORY_PAGE_SIZE_EXP_MIN,
        49
    );
}

#[test]
fn the_checks_are_read_at_each_worker_firmware_only() {
    assert_eq!(ALLOCATE_MEMORY_WORKERS.len(), 5);
    for (firmware, _) in ALLOCATE_MEMORY_WORKERS {
        assert!(allocate_memory_checks_read(firmware), "{firmware}");
    }
    assert!(!allocate_memory_checks_read("1.50"));
    assert!(!allocate_memory_checks_read("4.92"));
}

#[test]
fn the_4_93_worker_is_the_3_15_worker_moved_up_0x8180() {
    let at = |fw: &str| {
        ALLOCATE_MEMORY_WORKERS
            .iter()
            .find(|(f, _)| *f == fw)
            .map(|(_, a)| *a)
    };
    assert_eq!(at("4.93"), at("3.15").map(|a| a + 0x8180));
}
