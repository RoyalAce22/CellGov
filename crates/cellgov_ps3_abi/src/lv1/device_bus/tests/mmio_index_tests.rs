//! The index of each device of the MMIO regions, and the paths of lv1's
//! address function that the index selects.

use std::collections::BTreeSet;

use super::dma_slots_console_tests::{slot_object, slots};
use super::mmio_device_2_console_tests::{device_2, mmio_words};
use super::mmio_region::real_address_in;
use super::*;

/// A region that only these tests hold, for a device that no lv1 image
/// read shows.
const TEST_REGION: MmioRegion = MmioRegion {
    firmware: "test",
    bus: 1,
    device: 9,
    slot: 0,
    address: 0x2800,
    size: 0x200,
};

/// The index row of [`TEST_REGION`]'s device with the index `index`.
const fn test_index(index: u32) -> MmioDeviceIndex {
    MmioDeviceIndex {
        firmware: TEST_REGION.firmware,
        bus: TEST_REGION.bus,
        device: TEST_REGION.device,
        index,
    }
}

/// The answer for [`TEST_REGION`] over the index rows `indexes`.
fn test_answer(indexes: &[MmioDeviceIndex]) -> MmioRealAddress {
    let r = TEST_REGION;
    real_address_in(&[r], indexes, r.firmware, r.bus, r.device, r.address)
}

#[test]
fn each_device_index_is_the_r4_of_its_constructor_call_in_lv1s_init() {
    assert_eq!(
        MMIO_DEVICE_INDEXES,
        [
            // [OFW-4.93 lv1 a:0x3A9A4C] `li r4, 3`, before the regions of
            // device 1 [OFW-4.93 lv1 a:0x3A9A60-0x3A9A9C].
            MmioDeviceIndex {
                firmware: "4.93",
                bus: 1,
                device: 1,
                index: 3,
            },
            // [OFW-4.93 lv1 a:0x3A9AA0] `li r4, 4`, before the regions of
            // device 2 [OFW-4.93 lv1 a:0x3A9AB4-0x3A9AF0].
            MmioDeviceIndex {
                firmware: "4.93",
                bus: 1,
                device: 2,
                index: 4,
            },
        ]
    );
}

/// The constructor stores the index as the word at +128 of the device
/// [OFW-4.93 lv1 a:0x22DDD8] `stw r26, 128(r31)`.
#[test]
fn each_device_index_is_the_word_at_128_of_its_device_in_the_console_capture() {
    let device_1 = slots("wrapper.tsv")[&(slot_object() + 24)];
    let device_1_word = slots("device.tsv")[&(device_1 + 128)];
    let words = mmio_words();
    let device_2_word = words[&(device_2(&words) + 128)];
    let captured = [(1, device_1_word >> 32), (2, device_2_word >> 32)];
    assert_eq!(MMIO_DEVICE_INDEXES.len(), captured.len());
    for (entry, (device, index)) in MMIO_DEVICE_INDEXES.iter().zip(captured) {
        assert_eq!(
            (
                entry.firmware,
                entry.bus,
                entry.device,
                u64::from(entry.index)
            ),
            ("4.93", 1, device, index)
        );
    }
}

#[test]
fn each_device_of_the_mmio_regions_has_one_index_row() {
    let devices: BTreeSet<_> = MMIO_REGIONS
        .iter()
        .map(|r| (r.firmware, r.bus, r.device))
        .collect();
    let indexed: Vec<_> = MMIO_DEVICE_INDEXES
        .iter()
        .map(|e| (e.firmware, e.bus, e.device))
        .collect();
    assert_eq!(indexed.len(), devices.len());
    assert_eq!(indexed.iter().copied().collect::<BTreeSet<_>>(), devices);
}

/// For index 6, lv1 adds the doubleword that `ld r9, 480(r3)` loads from
/// the bus object
/// [OFW-4.93 lv1 a:0x22BE4C-0x22BE60].
#[test]
fn a_device_of_index_6_gets_the_bus_doubleword_refusal() {
    assert_eq!(MMIO_BUS_DOUBLEWORD_INDEX, 6);
    assert_eq!(
        test_answer(&[test_index(MMIO_BUS_DOUBLEWORD_INDEX)]),
        MmioRealAddress::BusDoubleword
    );
}

#[test]
fn a_device_with_no_index_row_gets_the_no_index_refusal() {
    assert_eq!(test_answer(&[]), MmioRealAddress::NoIndex);
    let other_device = MmioDeviceIndex {
        device: TEST_REGION.device + 1,
        ..test_index(3)
    };
    assert_eq!(test_answer(&[other_device]), MmioRealAddress::NoIndex);
}

/// Index 14 branches to the path of every index other than 6
/// [OFW-4.93 lv1 a:0x22BE1C-0x22BE24], which adds only
/// [`MMIO_REAL_BASE`] [OFW-4.93 lv1 a:0x22BE2C-0x22BE44].
#[test]
fn indexes_3_4_and_14_take_the_real_base_plus_the_bus_address() {
    for index in [3, 4, 14] {
        assert_eq!(
            test_answer(&[test_index(index)]),
            MmioRealAddress::Real(MMIO_REAL_BASE + TEST_REGION.address),
            "index {index}"
        );
    }
}
