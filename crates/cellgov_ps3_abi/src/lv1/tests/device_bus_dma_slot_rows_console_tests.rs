//! The five DMA regions of device 1 of bus 1, held in their order to the
//! console capture of the device's slot object and region list in lv1.

use super::dma_slots_console_tests::{listed_regions, slot_object, slots};
use super::*;
use crate::lv1::partition::LPAR_ID_GAME_OS;

fn device_1_rows() -> Vec<GameOsDmaRegion> {
    game_os_dma_regions("4.93", 1, 1).collect()
}

#[test]
fn the_device_1_rows_are_the_devices_listed_regions_in_list_order() {
    let rows = device_1_rows();
    let listed = listed_regions();
    assert_eq!(rows.len(), 5);
    assert_eq!(listed.len(), rows.len());
    for (place, (row, region)) in rows.iter().zip(&listed).enumerate() {
        let &(_, partition, address, size, page_shift) = region;
        assert_eq!(
            (partition, address, size, page_shift),
            (LPAR_ID_GAME_OS, row.address, row.size, row.page_shift),
            "region {place}"
        );
    }
}

/// The slot function takes the first free slot of
/// [`DMA_SLOT_COUNT`] [OFW-4.93 lv1 a:0x23D54C-0x23D578] and stores the
/// partition and the low word of the address
/// [OFW-4.93 lv1 a:0x23D5D0-0x23D5FC].
#[test]
fn each_device_1_row_holds_the_slot_of_its_place_and_the_next_slot_is_free() {
    let rows = device_1_rows();
    let object = slot_object();
    let (valid, low, partitions) = (
        slots("slot_valid.tsv"),
        slots("slot_address.tsv"),
        slots("wrapper.tsv"),
    );
    assert_eq!(rows.len(), 5);
    assert!(rows.len() < DMA_SLOT_COUNT);
    for (slot, row) in (0u64..).zip(&rows) {
        let mark = object + 104 + 16 * slot;
        assert_eq!(valid[&mark], 1, "slot {slot}");
        assert_eq!(low[&(mark + 4)], row.address & 0xFFFF_FFFF, "slot {slot}");
        assert_eq!(partitions[&(mark + 8)], LPAR_ID_GAME_OS, "slot {slot}");
    }
    assert_eq!(valid[&(object + 104 + 16 * 5)], 0);
}
