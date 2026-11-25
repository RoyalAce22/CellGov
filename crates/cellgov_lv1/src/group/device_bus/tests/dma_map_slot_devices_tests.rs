use super::*;
use crate::group::memory::region::MemoryRegion;
use crate::DmaRegion;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::{
    GameOsDmaMapping, GAME_OS_DMA_MAPPINGS, GAME_OS_DMA_REGIONS,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007D6B4].
const KERNEL_SC: u64 = 0x8000_0000_0007_D6B4;

/// The flags that the kernel's wrapper builds from a flags word of 0
/// [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc].
const KERNEL_FLAGS: u64 = 0xE000_0000_0000_0000;

/// A region of partition memory that holds the kernel's pages above the
/// real mode region.
const PAGE_MEMORY: MemoryRegion = MemoryRegion {
    lpar_address: 0x5000_0030_0000,
    size: 0x10_0000,
    page_size_exp: 16,
    unit_id: 0,
    counter: 5,
};

fn device(device: u64) -> OpenDevice {
    OpenDevice { bus: 1, device }
}

fn mapping(row: &GameOsDmaMapping) -> DmaMapping {
    DmaMapping {
        bus: row.bus,
        device: row.device,
        bus_address: row.bus_address,
        lpar_address: row.lpar_address,
        length: row.length,
    }
}

fn call(m: DmaMapping) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..6].copy_from_slice(&[
        m.bus,
        m.device,
        m.lpar_address,
        m.bus_address,
        m.length,
        KERNEL_FLAGS,
    ]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: MAP_DEVICE_DMA_REGION,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(40_737),
    }
}

/// The kernel's pages of devices 1 and 2, in the order of the calls.
fn slot_device_pages() -> Vec<DmaMapping> {
    GAME_OS_DMA_MAPPINGS
        .iter()
        .filter(|row| row.firmware == "4.93" && row.device != 0)
        .map(mapping)
        .collect()
}

/// The state with devices 1 and 2 open and their captured regions held.
fn slot_devices_allocated() -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::Opened(device(1)));
    state.apply(DeviceBusChange::Opened(device(2)));
    for row in GAME_OS_DMA_REGIONS.iter().filter(|row| row.device != 0) {
        state.apply(DeviceBusChange::DmaRegionAllocated(DmaRegion {
            bus: row.bus,
            device: row.device,
            address: row.address,
            size: row.size,
            page_shift: row.page_shift,
        }));
    }
    state
}

fn page_memory() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.create(PAGE_MEMORY);
    memory
}

fn map(state: &DeviceBusState, mapping: DmaMapping) -> HypercallDisposition {
    map_region(&call(mapping), Some("4.93"), state, &page_memory())
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn mapped_answer(mapping: DmaMapping) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::DmaRegionMapped(mapping));
    HypercallDisposition::Answered(answer)
}

#[test]
fn each_kernel_page_of_devices_1_and_2_maps_in_the_order_of_the_calls() {
    let pages = slot_device_pages();
    assert_eq!(pages.len(), 18);
    let mut state = slot_devices_allocated();
    for page in pages {
        assert_eq!(map(&state, page), mapped_answer(page), "{page:X?}");
        state.apply(DeviceBusChange::DmaRegionMapped(page));
    }
    assert_eq!(state.mappings_of(device(1)).count(), 9);
    assert_eq!(state.mappings_of(device(2)).count(), 9);
}

#[test]
fn a_page_that_device_1_holds_is_busy_for_device_1_and_free_for_device_2() {
    let first = slot_device_pages()[0];
    assert_eq!(first.device, 1);
    let mut state = slot_devices_allocated();
    state.apply(DeviceBusChange::DmaRegionMapped(first));
    assert_eq!(map(&state, first), answered(DMA_MAP_BUSY_R3));
    let other = DmaMapping { device: 2, ..first };
    assert_eq!(map(&state, other), mapped_answer(other));
}

#[test]
fn lv1s_own_region_of_device_0_is_no_region_of_devices_1_and_2() {
    let state = slot_devices_allocated();
    for id in [1, 2] {
        let page = DmaMapping {
            bus: 1,
            device: id,
            bus_address: 0xA000_0000,
            lpar_address: 0,
            length: 0x1000,
        };
        assert_eq!(
            map(&state, page),
            answered(DMA_MAP_NO_REGION_R3),
            "device {id}"
        );
    }
}

#[test]
fn a_page_of_a_closed_device_1_or_2_gets_lv1s_wrong_state() {
    let page = slot_device_pages()[0];
    let state = DeviceBusState::default();
    for id in [1, 2] {
        let call = DmaMapping { device: id, ..page };
        assert_eq!(
            map(&state, call),
            answered(DMA_MAP_NOT_OPEN_R3),
            "device {id}"
        );
    }
}
