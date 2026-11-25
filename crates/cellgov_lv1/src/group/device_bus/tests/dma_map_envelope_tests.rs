use super::*;
use crate::group::memory::region::MemoryRegion;
use crate::DmaRegion;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::{
    GameOsDmaMapping, GAME_OS_DMA_MAPPINGS, GAME_OS_DMA_MAP_FLAGS, GAME_OS_DMA_REGIONS,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007D6B4].
const KERNEL_SC: u64 = 0x8000_0000_0007_D6B4;

/// A region of partition memory that holds the kernel's pages above the
/// real mode region.
const PAGE_MEMORY: MemoryRegion = MemoryRegion {
    lpar_address: 0x5000_0030_0000,
    size: 0x10_0000,
    page_size_exp: 16,
    unit_id: 0,
    counter: 5,
};

/// The logical partition address of device 1's first captured page in
/// its region at 0xC0080000, from the capture `lv1_device_dma_mappings`.
const CAPTURED_LPAR: u64 = 0x5000_0032_0000;

/// The length of each captured page of devices 1 and 2.
const CAPTURED_PAGE: u64 = 0x1_0000;

fn mapping(row: &GameOsDmaMapping) -> DmaMapping {
    DmaMapping {
        bus: row.bus,
        device: row.device,
        bus_address: row.bus_address,
        lpar_address: row.lpar_address,
        length: row.length,
    }
}

fn call(m: DmaMapping, flags: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..6].copy_from_slice(&[
        m.bus,
        m.device,
        m.lpar_address,
        m.bus_address,
        m.length,
        flags,
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

/// The state with devices 0, 1 and 2 open and each captured region of the
/// kernel's boot held.
fn allocated() -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in [0, 1, 2] {
        state.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device }));
    }
    for row in GAME_OS_DMA_REGIONS
        .iter()
        .filter(|row| row.firmware == "4.93")
    {
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

fn memory() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.create(PAGE_MEMORY);
    memory
}

fn map(state: &DeviceBusState, request: &HypercallRequest) -> HypercallDisposition {
    map_region(request, Some("4.93"), state, &memory())
}

fn mapped_answer(mapping: DmaMapping) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::DmaRegionMapped(mapping));
    HypercallDisposition::Answered(answer)
}

fn uncaptured(request: &HypercallRequest) -> HypercallDisposition {
    UNCAPTURED_MAP.refusal(request)
}

/// The capture `lv1_device_dma_map` holds device 0's one mapping, and the
/// capture `lv1_device_dma_mappings` holds the 20 mappings of devices 1
/// and 2.
#[test]
fn each_captured_mapping_of_devices_0_1_and_2_maps_in_the_order_of_the_rows() {
    let rows: Vec<_> = GAME_OS_DMA_MAPPINGS
        .iter()
        .filter(|row| row.firmware == "4.93")
        .collect();
    assert_eq!(rows.len(), 21);
    let mut state = allocated();
    for row in rows {
        let m = mapping(row);
        assert_eq!(
            map(&state, &call(m, GAME_OS_DMA_MAP_FLAGS)),
            mapped_answer(m),
            "{row:?}"
        );
        state.apply(DeviceBusChange::DmaRegionMapped(m));
    }
    assert_eq!(state.dma_mappings().count(), 21);
}

#[test]
fn a_page_in_a_region_with_no_captured_mapping_is_a_refusal() {
    let page = |device, bus_address, lpar_address, length| DmaMapping {
        bus: 1,
        device,
        bus_address,
        lpar_address,
        length,
    };
    let calls = [
        page(1, 0xE000_0000, CAPTURED_LPAR, CAPTURED_PAGE),
        page(2, 0xE000_0000, CAPTURED_LPAR, CAPTURED_PAGE),
        page(2, 0xC080_0000, CAPTURED_LPAR, CAPTURED_PAGE),
        page(1, 0xD000_0000, 0x80_0000, 0x10_0000),
    ];
    assert_eq!(calls.len(), 4);
    for m in calls {
        let request = call(m, GAME_OS_DMA_MAP_FLAGS);
        assert_eq!(map(&allocated(), &request), uncaptured(&request), "{m:?}");
    }
}

#[test]
fn a_call_longer_than_the_captured_page_is_a_refusal_and_device_0_gets_the_length_check() {
    let longer = |device, lpar_address| DmaMapping {
        bus: 1,
        device,
        bus_address: 0xC008_0000,
        lpar_address,
        length: 2 * CAPTURED_PAGE,
    };
    for m in [longer(1, CAPTURED_LPAR), longer(2, CAPTURED_LPAR)] {
        let request = call(m, GAME_OS_DMA_MAP_FLAGS);
        assert_eq!(map(&allocated(), &request), uncaptured(&request), "{m:?}");
    }
    let network = DmaMapping {
        length: 0x20_0000,
        ..mapping(&GAME_OS_DMA_MAPPINGS[0])
    };
    assert_eq!(
        map(&allocated(), &call(network, GAME_OS_DMA_MAP_FLAGS)),
        lv1_answer(DMA_MAP_LENGTH_R3),
    );
}

#[test]
fn a_captured_call_of_each_device_with_other_allowed_flags_is_a_refusal() {
    let flags = [
        0,
        0x4000_0000_0000_0000,
        0x8000_0000_0000_0000,
        0xE000_0000_0000_0800,
        DMA_MAP_FLAGS_MASK,
    ];
    let mut checked = 0;
    for device in [0, 1, 2] {
        let row = GAME_OS_DMA_MAPPINGS
            .iter()
            .find(|row| row.firmware == "4.93" && row.device == device)
            .unwrap_or_else(|| panic!("a 4.93 row of device {device}"));
        for flags in flags {
            assert_eq!(flags & !DMA_MAP_FLAGS_MASK, 0, "lv1's flags check passes");
            assert_ne!(flags, GAME_OS_DMA_MAP_FLAGS);
            let request = call(mapping(row), flags);
            assert_eq!(
                map(&allocated(), &request),
                uncaptured(&request),
                "{row:?} {flags:#x}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 15);
}

#[test]
fn a_device_0_call_at_another_lpar_address_is_a_refusal() {
    let captured = mapping(&GAME_OS_DMA_MAPPINGS[0]);
    assert_eq!(captured.device, 0);
    let moved = DmaMapping {
        lpar_address: captured.lpar_address + 0x10_0000,
        ..captured
    };
    let request = call(moved, GAME_OS_DMA_MAP_FLAGS);
    assert_eq!(map(&allocated(), &request), uncaptured(&request));
}
