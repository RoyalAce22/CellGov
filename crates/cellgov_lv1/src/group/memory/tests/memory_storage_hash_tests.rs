use super::*;

#[test]
fn a_stored_region_adds_its_real_address_under_the_storage_source() {
    let region = MemoryRegion {
        lpar_address: 0x0000_5000_0030_0000,
        size: 0xA_0000,
        page_size_exp: 16,
        unit_id: 1,
        counter: 3,
    };
    let mut memory = MemoryState::default();
    memory.create(region);
    let expected = lanes::value_term(REGION_COUNTER_SOURCE, 0, &4u64)
        .wrapping_add(lanes::value_term(REGION_SOURCE, 0, &region))
        .wrapping_add(lanes::value_term(
            REGION_STORAGE_SOURCE,
            0,
            &ALLOCATED_STORAGE_BASE,
        ));
    assert_eq!(memory.sync_term(), expected);
}

#[test]
fn a_region_with_no_storage_adds_no_storage_term() {
    let region = MemoryRegion {
        lpar_address: 0x0000_6C00_1000_0000,
        size: 0x790_0000,
        page_size_exp: 20,
        unit_id: 1,
        counter: 2,
    };
    let mut memory = MemoryState::default();
    memory.create(region);
    let expected = lanes::value_term(REGION_COUNTER_SOURCE, 0, &3u64)
        .wrapping_add(lanes::value_term(REGION_SOURCE, 0, &region));
    assert_eq!(memory.sync_term(), expected);
}
