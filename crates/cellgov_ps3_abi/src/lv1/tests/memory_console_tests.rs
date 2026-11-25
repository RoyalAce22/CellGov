//! The region addresses held to the console capture of the 4.93 boot
//! repository record. The README of the capture gives the offsets.

use super::*;
use crate::lv1::boot_record_capture::captured_doubleword;
use crate::lv1::repository::BOOT_RECORD_4_93;

/// The size of the first allocation: `bi.rgntotal` less `rm_size` less
/// the low 20 bits of `bi.rgntotal`.
const FIRST_SIZE: u64 = 0x90;
/// The address the first allocation returned in r4.
const FIRST_ADDRESS: u64 = 0x88;
/// The low 20 bits of `bi.rgntotal`. The kernel clears the low 16 bits
/// of this word and passes the result as the size of the second
/// allocation.
const SECOND_SIZE: u64 = 0xa0;
/// The address the second allocation returned in r4.
const SECOND_ADDRESS: u64 = 0x98;

fn record(offset: u64) -> u64 {
    captured_doubleword(BOOT_RECORD_4_93 + offset)
}

/// The r3 of the allocation whose size word sits at `offset`.
fn passed_size(offset: u64) -> u64 {
    match offset {
        SECOND_SIZE => record(offset) & !0xffff,
        _ => record(offset),
    }
}

#[test]
fn the_4_93_boot_allocations_take_the_boot_counter_and_the_next_value() {
    let counter = game_os_region_counter_at_boot("4.93").expect("4.93 counter");
    assert_eq!(
        region_address(passed_size(FIRST_SIZE), counter),
        Some(record(FIRST_ADDRESS))
    );
    assert_eq!(
        region_address(passed_size(SECOND_SIZE), counter + 1),
        Some(record(SECOND_ADDRESS))
    );
}

#[test]
fn the_4_93_boot_allocation_addresses_hold_rounded_up_size_exponents() {
    for (size, address) in [(FIRST_SIZE, FIRST_ADDRESS), (SECOND_SIZE, SECOND_ADDRESS)] {
        let (size, address) = (passed_size(size), record(address));
        assert!(!size.is_power_of_two(), "{size:#x}");
        assert_eq!(
            Some(address >> REGION_SIZE_EXP_SHIFT),
            region_size_exp(size).map(u64::from),
            "{size:#x}"
        );
    }
}
