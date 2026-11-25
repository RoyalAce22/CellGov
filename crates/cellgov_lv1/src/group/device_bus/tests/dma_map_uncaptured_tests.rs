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
/// real mode region, and the free page at +0x80000.
const PAGE_MEMORY: MemoryRegion = MemoryRegion {
    lpar_address: 0x5000_0030_0000,
    size: 0x10_0000,
    page_size_exp: 16,
    unit_id: 0,
    counter: 5,
};

/// The first captured row of `device`.
fn first_row(device: u64) -> GameOsDmaMapping {
    let row = GAME_OS_DMA_MAPPINGS
        .iter()
        .find(|row| row.firmware == "4.93" && row.device == device)
        .copied();
    row.unwrap_or_else(|| panic!("a 4.93 row of device {device}"))
}

fn mapping(row: GameOsDmaMapping) -> DmaMapping {
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

/// The state with devices 1 and 2 open and their captured regions held.
fn slot_devices_allocated() -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in [1, 2] {
        state.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device }));
    }
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

fn map(request: &HypercallRequest) -> HypercallDisposition {
    let mut memory = MemoryState::default();
    memory.create(PAGE_MEMORY);
    map_region(request, Some("4.93"), &slot_devices_allocated(), &memory)
}

fn mapped_answer(mapping: DmaMapping) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::DmaRegionMapped(mapping));
    HypercallDisposition::Answered(answer)
}

fn uncaptured(request: &HypercallRequest) -> HypercallDisposition {
    UNCAPTURED_MAP.refusal(request)
}

#[test]
fn a_device_1_page_that_passes_the_checks_at_an_uncaptured_lpar_address_is_a_refusal() {
    let captured = mapping(first_row(1));
    assert_eq!(
        map(&call(captured, GAME_OS_DMA_MAP_FLAGS)),
        mapped_answer(captured)
    );
    let moved = DmaMapping {
        lpar_address: PAGE_MEMORY.lpar_address + 0x8_0000,
        ..captured
    };
    assert_ne!(moved.lpar_address, captured.lpar_address);
    let request = call(moved, GAME_OS_DMA_MAP_FLAGS);
    assert_eq!(map(&request), uncaptured(&request));
}

#[test]
fn a_captured_page_of_device_2_with_other_flags_is_a_refusal() {
    let captured = mapping(first_row(2));
    let flags = 0x8000_0000_0000_0000;
    assert_eq!(flags & !DMA_MAP_FLAGS_MASK, 0, "lv1's flags check passes");
    assert_ne!(flags, GAME_OS_DMA_MAP_FLAGS);
    let request = call(captured, flags);
    assert_eq!(map(&request), uncaptured(&request));
}
