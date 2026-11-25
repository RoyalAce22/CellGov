use super::*;
use crate::answer::HypercallRefusal;
use crate::group::memory::region::MemoryRegion;
use crate::{answered_numbers, DmaRegion, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{ALLOCATE_DEVICE_DMA_REGION, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::{
    DMA_MAP_BUS_ALIGNMENT_R3, DMA_MAP_LPAR_ALIGNMENT_R3, GAME_OS_DMA_MAPPINGS, GAME_OS_DMA_REGIONS,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007D6B4].
const KERNEL_SC: u64 = 0x8000_0000_0007_D6B4;

/// The flags that the kernel's wrapper builds from a flags word of 0
/// [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc].
const KERNEL_FLAGS: u64 = 0xE000_0000_0000_0000;

/// The region that the 4.93 kernel's allocate call gets, from the console
/// capture `lv1_device_dma_region`.
const KERNEL_REGION: DmaRegion = {
    let row = GAME_OS_DMA_REGIONS[0];
    DmaRegion {
        bus: row.bus,
        device: row.device,
        address: row.address,
        size: row.size,
        page_shift: row.page_shift,
    }
};

/// A second region of the network device, above the kernel's region.
const OTHER_REGION: DmaRegion = DmaRegion {
    address: 0xD000_0000,
    ..KERNEL_REGION
};

/// The call with r3 to r8.
fn request(
    bus: u64,
    device: u64,
    lpar_address: u64,
    bus_address: u64,
    length: u64,
    flags: u64,
) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..6].copy_from_slice(&[bus, device, lpar_address, bus_address, length, flags]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: MAP_DEVICE_DMA_REGION,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(32_934),
    }
}

/// The mapping of the console's record of the kernel's call.
fn kernel_mapping() -> DmaMapping {
    let row = GAME_OS_DMA_MAPPINGS[0];
    DmaMapping {
        bus: row.bus,
        device: row.device,
        bus_address: row.bus_address,
        lpar_address: row.lpar_address,
        length: row.length,
    }
}

/// The kernel's call, with the inputs of the console's record.
fn kernel_call() -> HypercallRequest {
    let m = kernel_mapping();
    request(
        m.bus,
        m.device,
        m.lpar_address,
        m.bus_address,
        m.length,
        KERNEL_FLAGS,
    )
}

fn network(lpar_address: u64, bus_address: u64, length: u64) -> HypercallRequest {
    request(1, 0, lpar_address, bus_address, length, KERNEL_FLAGS)
}

/// The state with the network device open.
fn opened() -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    state
}

/// The state after the kernel's allocate call.
fn allocated() -> DeviceBusState {
    let mut state = opened();
    state.apply(DeviceBusChange::DmaRegionAllocated(KERNEL_REGION));
    state
}

/// The state after the kernel's map call.
fn mapped() -> DeviceBusState {
    let mut state = allocated();
    state.apply(DeviceBusChange::DmaRegionMapped(kernel_mapping()));
    state
}

fn map(state: &DeviceBusState, request: &HypercallRequest) -> HypercallDisposition {
    map_region(request, Some("4.93"), state, &MemoryState::default())
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn mapped_answer(mapping: DmaMapping) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::DmaRegionMapped(mapping));
    HypercallDisposition::Answered(answer)
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: MAP_DEVICE_DMA_REGION,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

fn r3_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) => Some(answer.r3),
        HypercallDisposition::Refused(_) => None,
    }
}

/// [OFW-4.93 lv1 a:0x227CC4-0x227CD0] returns 0; the handler writes no
/// return slot [OFW-4.93 lv1 a:0x3068A4].
#[test]
fn the_kernel_call_gets_0_and_carries_the_console_mapping() {
    assert_eq!(
        map(&allocated(), &kernel_call()),
        mapped_answer(kernel_mapping())
    );
}

/// [OFW-4.93 lv1 a:0x306818-0x30683C] checks the bus and then the flags;
/// [OFW-4.93 lv1 a:0x2363E0] refuses bus 0 and then a device that the
/// partition did not open.
#[test]
fn each_handler_and_access_check_gets_its_status_before_the_region_lookup() {
    let state = DeviceBusState::default();
    let cases = [
        (request(5, 0, 0, 0, 0, 1), DMA_MAP_BUS_RANGE_R3),
        (request(1 << 32, 0, 0, 0, 0, 0), DMA_MAP_BUS_RANGE_R3),
        (request(1, 0, 0, 0, 0, 1), DMA_MAP_FLAGS_R3),
        (request(1, 0, 0, 0, 0, 0x1000), DMA_MAP_FLAGS_R3),
        (
            request(1, 0, 0, 0, 0, 0x0400_0000_0000_0000),
            DMA_MAP_FLAGS_R3,
        ),
        (request(0, 7, 0, 0, 0, 1), DMA_MAP_FLAGS_R3),
        (request(0, 7, 0, 0, 0, 0), DMA_MAP_NO_BUS_R3),
        (
            request(1, 0, 0, 0, 0, DMA_MAP_FLAGS_MASK),
            DMA_MAP_NOT_OPEN_R3,
        ),
        (kernel_call(), DMA_MAP_NOT_OPEN_R3),
    ];
    assert_eq!(cases.len(), 9);
    for (call, r3) in cases {
        assert_eq!(map(&state, &call), answered(r3), "{:?}", call.inputs);
    }
}

/// [OFW-4.93 lv1 a:0x22799C-0x2279D0] finds no region;
/// [OFW-4.93 lv1 a:0x227444-0x22744C] answers -6 with r6 marked.
#[test]
fn a_bus_address_in_no_region_of_the_device_gets_r6_marked() {
    let cases = [
        (allocated(), network(0x80_0000, 0xC010_0000, 0x10_0000)),
        (allocated(), network(0x80_0000, 0xBFF0_0000, 0x10_0000)),
        (allocated(), network(0, 0xA000_8000, 0x1000)),
        (allocated(), network(0, 0x9FFF_F000, 0x1000)),
        (opened(), kernel_call()),
    ];
    assert_eq!(cases.len(), 5);
    for (state, call) in cases {
        assert_eq!(
            map(&state, &call),
            answered(DMA_MAP_NO_REGION_R3),
            "{:?}",
            call.inputs
        );
    }
}

/// [OFW-4.93 lv1 a:0x2272BC-0x2272CC] checks the length before the range
/// check, in a region of either owner.
#[test]
fn a_length_of_0_or_above_the_region_gets_r7_marked() {
    let cases = [
        network(0x80_0000, 0xC000_0000, 0),
        network(0x80_0000, 0xC000_0000, 0x10_1000),
        network(0x80_0000, 0xC00F_F000, u64::MAX),
        network(0, 0xA000_0000, 0),
        network(0, 0xA000_0000, 0x9000),
    ];
    assert_eq!(cases.len(), 5);
    for call in cases {
        assert_eq!(
            map(&mapped(), &call),
            answered(DMA_MAP_LENGTH_R3),
            "{:?}",
            call.inputs
        );
    }
}

/// [OFW-4.93 lv1 a:0x2257D4-0x2257E4] bounds the range by the region;
/// [OFW-4.93 lv1 a:0x2257E8-0x22583C] tests the region's mappings.
#[test]
fn a_range_past_the_region_or_over_a_mapping_gets_busy_with_r6_marked() {
    let cases = [
        (allocated(), network(0x80_0000, 0xC00F_F000, 0x2000)),
        (mapped(), kernel_call()),
        (mapped(), network(0x90_0000, 0xC000_0000, 0x10_0000)),
    ];
    assert_eq!(cases.len(), 3);
    for (state, call) in cases {
        assert_eq!(
            map(&state, &call),
            answered(DMA_MAP_BUSY_R3),
            "{:?}",
            call.inputs
        );
    }
}

#[test]
fn a_mapping_of_another_region_does_not_make_a_call_busy() {
    let mut state = mapped();
    state.apply(DeviceBusChange::DmaRegionAllocated(OTHER_REGION));
    let call = network(0x90_0000, OTHER_REGION.address, 0x10_0000);
    let mapping = DmaMapping {
        bus_address: OTHER_REGION.address,
        lpar_address: 0x90_0000,
        ..kernel_mapping()
    };
    assert_eq!(map(&state, &call), mapped_answer(mapping));
}

/// [OFW-4.93 lv1 a:0x22732C-0x227358] checks each value against the
/// region's 1 MB page.
#[test]
fn each_page_check_gets_its_marked_status() {
    let cases = [
        (
            network(0x80_0000, 0xC000_1000, 0x1000),
            DMA_MAP_BUS_ALIGNMENT_R3,
        ),
        (
            network(0x80_1000, 0xC000_0000, 0x10_0000),
            DMA_MAP_LPAR_ALIGNMENT_R3,
        ),
        (network(0x80_0000, 0xC000_0000, 0x8_0000), DMA_MAP_LENGTH_R3),
    ];
    assert_eq!(cases.len(), 3);
    for (call, r3) in cases {
        assert_eq!(map(&allocated(), &call), answered(r3), "{:?}", call.inputs);
    }
}

#[test]
fn the_page_checks_use_the_page_of_the_region_that_holds_the_bus_address() {
    let small_pages = DmaRegion {
        page_shift: 12,
        ..OTHER_REGION
    };
    let mut state = allocated();
    state.apply(DeviceBusChange::DmaRegionAllocated(small_pages));
    let call = network(0x80_1000, small_pages.address + 0x1000, 0x1000);
    let mapping = DmaMapping {
        bus_address: small_pages.address + 0x1000,
        lpar_address: 0x80_1000,
        length: 0x1000,
        ..kernel_mapping()
    };
    assert_eq!(map(&state, &call), mapped_answer(mapping));
}

/// [OFW-4.93 lv1 a:0x2257E8-0x22583C] tests each mapping of the region by
/// its offset in the region.
#[test]
fn a_call_that_starts_where_a_held_mapping_ends_maps() {
    let small_pages = DmaRegion {
        page_shift: 12,
        ..OTHER_REGION
    };
    let mut state = allocated();
    state.apply(DeviceBusChange::DmaRegionAllocated(small_pages));
    state.apply(DeviceBusChange::DmaRegionMapped(DmaMapping {
        bus_address: small_pages.address,
        lpar_address: 0x80_0000,
        length: 0x1000,
        ..kernel_mapping()
    }));
    let call = network(0x80_1000, small_pages.address + 0x1000, 0x1000);
    let mapping = DmaMapping {
        bus_address: small_pages.address + 0x1000,
        lpar_address: 0x80_1000,
        length: 0x1000,
        ..kernel_mapping()
    };
    assert_eq!(map(&state, &call), mapped_answer(mapping));
}

#[test]
fn a_call_for_another_bus_or_device_of_bus_1_is_a_refusal() {
    let cases = [
        (
            request(2, 0, 0x80_0000, 0xC000_0000, 0x10_0000, 0),
            OTHER_BUS.what,
        ),
        (
            request(4, 0, 0x80_0000, 0xC000_0000, 0x10_0000, 0),
            OTHER_BUS.what,
        ),
        (
            request(1, 1, 0x80_0000, 0xC000_0000, 0x10_0000, 0),
            OTHER_DEVICE.what,
        ),
        (request(1, 1 << 32, 0, 0, 0, 0), OTHER_DEVICE.what),
    ];
    assert_eq!(cases.len(), 4);
    for (call, what) in cases {
        assert_eq!(
            map(&allocated(), &call),
            refused(&call, what),
            "{:?}",
            call.inputs
        );
    }
}

#[test]
fn a_call_in_lv1s_own_region_past_the_length_check_is_a_refusal() {
    let calls = [
        network(0, 0xA000_0000, 0x1000),
        network(0, 0xA000_7000, 0x8000),
    ];
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(
            map(&allocated(), &call),
            refused(&call, LV1_REGION.what),
            "{:?}",
            call.inputs
        );
    }
}

#[test]
fn memory_past_the_real_mode_region_is_a_refusal_unless_a_region_holds_it() {
    let rm_size = real_mode_size("4.93").expect("the 4.93 real mode size");
    let last = network(rm_size - 0x10_0000, 0xC000_0000, 0x10_0000);
    assert_eq!(r3_of(&map(&allocated(), &last)), Some(0));
    let past = network(rm_size, 0xC000_0000, 0x10_0000);
    assert_eq!(
        map(&allocated(), &past),
        refused(&past, OUTSIDE_MEMORY.what)
    );
    let wrap = network(0xFFFF_FFFF_FFF0_0000, 0xC000_0000, 0x10_0000);
    assert_eq!(
        map(&allocated(), &wrap),
        refused(&wrap, OUTSIDE_MEMORY.what)
    );

    let base = 0x1_0000_0000_0000;
    let mut memory = MemoryState::default();
    memory.create(MemoryRegion {
        lpar_address: base,
        size: 0x20_0000,
        page_size_exp: 20,
        unit_id: 0,
        counter: 0,
    });
    let first = network(base, 0xC000_0000, 0x10_0000);
    assert_eq!(
        r3_of(&map_region(&first, Some("4.93"), &allocated(), &memory)),
        Some(0)
    );
    let held = network(base + 0x10_0000, 0xC000_0000, 0x10_0000);
    assert_eq!(
        r3_of(&map_region(&held, Some("4.93"), &allocated(), &memory)),
        Some(0)
    );
    let below = network(base - 0x10_0000, 0xC000_0000, 0x10_0000);
    assert_eq!(
        map_region(&below, Some("4.93"), &allocated(), &memory),
        refused(&below, OUTSIDE_MEMORY.what)
    );
    let over = network(base + 0x20_0000, 0xC000_0000, 0x10_0000);
    assert_eq!(
        map_region(&over, Some("4.93"), &allocated(), &memory),
        refused(&over, OUTSIDE_MEMORY.what)
    );
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let call = kernel_call();
    for firmware in [None, Some("4.92"), Some("3.70")] {
        assert_eq!(
            map_region(&call, firmware, &allocated(), &MemoryState::default()),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_176_after_the_allocate_and_holds_the_mapping() {
    assert!(answered_numbers().contains(&MAP_DEVICE_DMA_REGION));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(1, 0, 0, 0, 0, 0)
    };
    assert_eq!(r3_of(&hv.dispatch(&open)), Some(0));
    let mut allocate = request(1, 0, 0x10_0000, 20, 0, 0);
    allocate.number = ALLOCATE_DEVICE_DMA_REGION;
    assert_eq!(r3_of(&hv.dispatch(&allocate)), Some(0));
    let before = hv.sync_term();
    let disposition = hv.dispatch(&kernel_call());
    assert_eq!(r3_of(&disposition), Some(0));
    let outputs = match disposition {
        HypercallDisposition::Answered(answer) => answer.outputs(),
        HypercallDisposition::Refused(_) => None,
    };
    assert_eq!(outputs, Some([0; 7]));
    let mappings: Vec<_> = hv.groups().device_bus().dma_mappings().collect();
    assert_eq!(mappings, [kernel_mapping()]);
    assert_ne!(hv.sync_term(), before);
}
