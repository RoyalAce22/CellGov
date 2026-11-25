use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn region(lpar_address: u64, size: u64, page_size_exp: u8, counter: u64) -> MemoryRegion {
    MemoryRegion {
        lpar_address,
        size,
        page_size_exp,
        unit_id: 1,
        counter,
    }
}

#[test]
fn a_memory_state_with_no_region_adds_no_term() {
    assert_eq!(MemoryState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with two regions.
#[test]
fn memory_sync_term_lane_vector_golden() {
    let mut memory = MemoryState::default();
    memory.create(region(0x6C00_1000_0000, 0x790_0000, 20, 2));
    memory.create(region(0x5000_0030_0000, 0xA_0000, 16, 3));
    let lane =
        |src, object, field, value| contribution(LaneIndex::new(src, object, field, 0), value);
    let lanes = [
        lane(REGION_COUNTER_SOURCE, 0, 0, 1),
        lane(REGION_COUNTER_SOURCE, 0, 1, 4),
        lane(REGION_SOURCE, 0, 0, 1),
        lane(REGION_SOURCE, 0, 1, 0x6C00_1000_0000),
        lane(REGION_SOURCE, 0, 2, 0x790_0000),
        lane(REGION_SOURCE, 0, 3, 20),
        lane(REGION_SOURCE, 0, 4, 1),
        lane(REGION_SOURCE, 0, 5, 2),
        lane(REGION_SOURCE, 1, 0, 1),
        lane(REGION_SOURCE, 1, 1, 0x5000_0030_0000),
        lane(REGION_SOURCE, 1, 2, 0xA_0000),
        lane(REGION_SOURCE, 1, 3, 16),
        lane(REGION_SOURCE, 1, 4, 1),
        lane(REGION_SOURCE, 1, 5, 3),
        lane(REGION_STORAGE_SOURCE, 1, 0, 1),
        lane(REGION_STORAGE_SOURCE, 1, 1, ALLOCATED_STORAGE_BASE),
    ];
    let sum = lanes.into_iter().fold(0u128, u128::wrapping_add);
    assert_eq!(memory.sync_term(), sum);
}
