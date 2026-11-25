use super::*;
use crate::lv1::call::{lookup, CallSource, ALLOCATE_MEMORY};

/// Linux `lv1call.h` lists `allocate_memory` as number 0 with 4 inputs
/// and 2 outputs.
#[test]
fn allocate_memory_carries_its_header_number_name_and_counts() {
    assert_eq!(ALLOCATE_MEMORY, 0);
    let row = lookup(ALLOCATE_MEMORY).map(|c| (c.name, c.inputs, c.outputs, c.source));
    assert_eq!(row, Some(("allocate_memory", 4, 2, CallSource::Linux)));
}

/// Each row is a region the page lists with its size and address
/// [PS3DEVWIKI-HV-RE s:Physical_Memory_Region_class]: a 4 KiB, a 128 KiB
/// and a 112 MiB region of HV 3.15. The last row is the 3.41 GameOS
/// region [PS3DEVWIKI-HV-RE s:GameOS_Physical_Memory_Regions].
#[test]
fn the_address_of_each_listed_region_is_its_size_exponent_and_counter() {
    let regions: [(u64, u64, u64); 4] = [
        (0x1000, 1, 0x3000_0000_1000),
        (0x2_0000, 2, 0x4400_0004_0000),
        (0x700_0000, 11, 0x6C00_5800_0000),
        (0xE90_0000, 2, 0x7000_2000_0000),
    ];
    for (size, counter, address) in regions {
        assert_eq!(region_address(size, counter), Some(address), "{size:#x}");
        assert_eq!(
            address >> REGION_SIZE_EXP_SHIFT,
            u64::from(region_size_exp(size).expect("nonzero size")),
            "{size:#x}"
        );
    }
}

#[test]
fn the_size_exponent_rounds_up_between_powers_of_two() {
    assert_eq!(region_size_exp(0), None);
    assert_eq!(region_size_exp(1), Some(0));
    assert_eq!(region_size_exp(0x10_0000), Some(20));
    assert_eq!(region_size_exp(0x10_0001), Some(21));
    assert_eq!(region_size_exp(u64::MAX), Some(64));
}

#[test]
fn a_region_with_no_size_an_exponent_of_42_or_a_counter_that_reaches_bit_42_has_no_address() {
    assert_eq!(region_address(0, 2), None);
    // 2^20-byte regions: counter values below 2^22 fit below bit 42.
    assert_eq!(
        region_address(0x10_0000, (1 << 22) - 1),
        Some((20 << 42) | (((1 << 22) - 1) << 20))
    );
    assert_eq!(region_address(0x10_0000, 1 << 22), None);
    assert_eq!(region_address(1 << 42, 0), None);
}

#[test]
fn only_4_93_has_a_boot_region_counter() {
    assert_eq!(GAME_OS_REGION_COUNTER_AT_BOOT.len(), 1);
    assert!(game_os_region_counter_at_boot("4.93").is_some());
    for fw in ["1.50", "1.94", "3.41", "3.70", "4.92"] {
        assert_eq!(game_os_region_counter_at_boot(fw), None, "{fw}");
    }
}
