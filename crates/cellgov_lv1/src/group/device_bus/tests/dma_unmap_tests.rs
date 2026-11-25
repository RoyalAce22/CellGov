use super::super::dma_map::map_region;
use super::*;
use crate::answer::HypercallRefusal;
use crate::group::memory::region::MemoryState;
use crate::{answered_numbers, DmaMapping, DmaRegion, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{ALLOCATE_DEVICE_DMA_REGION, MAP_DEVICE_DMA_REGION, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::{
    GameOsDmaMapping, GAME_OS_DMA_MAPPINGS, GAME_OS_DMA_REGIONS,
};
use cellgov_time::GuestTicks;

/// The first `sc` of the 4.93 kernel's unmap wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007d478].
const KERNEL_SC: u64 = 0x8000_0000_0007_D478;

/// The `sc` of the 4.93 kernel's map wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007D6B4].
const MAP_SC: u64 = 0x8000_0000_0007_D6B4;

/// The flags that the kernel's map wrapper builds from a flags word of 0
/// [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc].
const KERNEL_FLAGS: u64 = 0xE000_0000_0000_0000;

const NETWORK: OpenDevice = OpenDevice { bus: 1, device: 0 };

fn request(number: u64, pc: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(41_002),
    }
}

/// The unmap call with r3 to r6.
fn unmap_call(bus: u64, device: u64, bus_address: u64, length: u64) -> HypercallRequest {
    request(
        UNMAP_DEVICE_DMA_REGION,
        KERNEL_SC,
        &[bus, device, bus_address, length],
    )
}

/// The unmap call that names the first byte and the length of `m`.
fn unmap_of(m: DmaMapping) -> HypercallRequest {
    unmap_call(m.bus, m.device, m.bus_address, m.length)
}

fn map_of(m: DmaMapping) -> HypercallRequest {
    request(
        MAP_DEVICE_DMA_REGION,
        MAP_SC,
        &[
            m.bus,
            m.device,
            m.lpar_address,
            m.bus_address,
            m.length,
            KERNEL_FLAGS,
        ],
    )
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

/// The mapping of the console's record of the kernel's map of the
/// network device.
fn network_mapping() -> DmaMapping {
    mapping(&GAME_OS_DMA_MAPPINGS[0])
}

/// The state with each device of the console's records open, its
/// captured regions held, and each mapping of `rows` made.
fn mapped(rows: &[GameOsDmaMapping]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in DMA_MAP_DEVICES {
        state.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device }));
    }
    for row in GAME_OS_DMA_REGIONS {
        state.apply(DeviceBusChange::DmaRegionAllocated(DmaRegion {
            bus: row.bus,
            device: row.device,
            address: row.address,
            size: row.size,
            page_shift: row.page_shift,
        }));
    }
    for row in rows {
        state.apply(DeviceBusChange::DmaRegionMapped(mapping(row)));
    }
    state
}

fn unmap(state: &DeviceBusState, call: &HypercallRequest) -> HypercallDisposition {
    unmap_region(call, Some("4.93"), state)
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn unmapped_answer(mapping: DmaMapping) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::DmaRegionUnmapped(mapping));
    HypercallDisposition::Answered(answer)
}

fn refused(call: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: UNMAP_DEVICE_DMA_REGION,
        unit: call.unit,
        pc: call.pc,
        what,
    })
}

fn r3_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) => Some(answer.r3),
        HypercallDisposition::Refused(_) => None,
    }
}

/// [OFW-4.93 lv1 a:0x227AAC-0x227AC4] unlinks the mapping and returns 0;
/// the handler writes no return slot [OFW-4.93 lv1 a:0x306738].
#[test]
fn an_unmap_of_the_network_mapping_gets_0_and_carries_its_removal() {
    let state = mapped(&GAME_OS_DMA_MAPPINGS[..1]);
    assert_eq!(
        unmap(&state, &unmap_of(network_mapping())),
        unmapped_answer(network_mapping())
    );
}

#[test]
fn after_the_unmap_the_same_range_maps_again() {
    let mut state = mapped(&GAME_OS_DMA_MAPPINGS[..1]);
    let HypercallDisposition::Answered(answer) = unmap(&state, &unmap_of(network_mapping())) else {
        panic!("the unmap is not answered");
    };
    assert_eq!(answer.changes.len(), 1);
    state.apply(DeviceBusChange::DmaRegionUnmapped(network_mapping()));
    assert_eq!(state.dma_mappings().count(), 0);
    let remap = map_region(
        &map_of(network_mapping()),
        Some("4.93"),
        &state,
        &MemoryState::default(),
    );
    assert_eq!(r3_of(&remap), Some(0));
}

/// The kernel's unmap wrapper sends one call for each page
/// [OFW-4.93 lv2_kernel a:0x800000000007d470-0x800000000007d4c4].
#[test]
fn each_captured_page_of_devices_1_and_2_unmaps_and_leaves_the_others() {
    let pages: Vec<_> = GAME_OS_DMA_MAPPINGS
        .iter()
        .filter(|row| row.device != 0 && row.length == 0x1_0000)
        .map(mapping)
        .collect();
    assert_eq!(pages.len(), 19);
    let mut state = mapped(GAME_OS_DMA_MAPPINGS);
    let held = state.dma_mappings().count();
    assert_eq!(held, GAME_OS_DMA_MAPPINGS.len());
    for (unmapped, page) in pages.into_iter().enumerate() {
        assert_eq!(unmap(&state, &unmap_of(page)), unmapped_answer(page));
        state.apply(DeviceBusChange::DmaRegionUnmapped(page));
        assert_eq!(state.dma_mappings().count(), held - unmapped - 1);
    }
    let left: Vec<_> = state.dma_mappings().collect();
    let last = mapping(&GAME_OS_DMA_MAPPINGS[GAME_OS_DMA_MAPPINGS.len() - 1]);
    assert_eq!(left, [network_mapping(), last]);
}

#[test]
fn an_unmap_for_device_2_leaves_device_1s_page_at_the_same_bus_address() {
    let first = mapping(&GAME_OS_DMA_MAPPINGS[1]);
    assert_eq!(first.device, 1);
    let other = DmaMapping { device: 2, ..first };
    let mut state = mapped(&GAME_OS_DMA_MAPPINGS[1..]);
    assert_eq!(unmap(&state, &unmap_of(other)), unmapped_answer(other));
    state.apply(DeviceBusChange::DmaRegionUnmapped(other));
    let device_1 = OpenDevice { bus: 1, device: 1 };
    assert!(state.mappings_of(device_1).any(|held| held == first));
    assert_eq!(
        state.mappings_of(OpenDevice { bus: 1, device: 2 }).count(),
        9
    );
}

/// [OFW-4.93 lv1 a:0x3066A4-0x3066A8] checks the bus;
/// [OFW-4.93 lv1 a:0x2363E0] refuses bus 0 and then a device that the
/// partition did not open.
#[test]
fn each_handler_and_access_check_gets_its_status_before_the_mapping_lookup() {
    let none = DeviceBusState::default();
    let m = network_mapping();
    let cases = [
        (
            unmap_call(5, 0, m.bus_address, m.length),
            DMA_UNMAP_BUS_RANGE_R3,
        ),
        (unmap_call(1 << 32, 0, 0, 0), DMA_UNMAP_BUS_RANGE_R3),
        (unmap_call(u64::MAX, 7, 0, 0), DMA_UNMAP_BUS_RANGE_R3),
        (unmap_call(0, 7, 0, 0), DMA_UNMAP_NO_BUS_R3),
        (unmap_of(m), DMA_UNMAP_NOT_OPEN_R3),
        (
            unmap_call(1, 2, 0xC008_0000, 0x1_0000),
            DMA_UNMAP_NOT_OPEN_R3,
        ),
        (unmap_call(1, 0, 0xA000_0000, 0x1000), DMA_UNMAP_NOT_OPEN_R3),
    ];
    assert_eq!(cases.len(), 7);
    for (call, r3) in cases {
        assert_eq!(unmap(&none, &call), answered(r3), "{:?}", call.inputs);
    }
}

/// [OFW-4.93 lv1 a:0x22709C] finds no mapping;
/// [OFW-4.93 lv1 a:0x226680-0x22668C] answers -6 with r5 marked.
#[test]
fn a_bus_address_that_no_mapping_holds_gets_r5_marked() {
    let m = network_mapping();
    let cases = [
        (mapped(&[]), unmap_of(m)),
        (
            mapped(&GAME_OS_DMA_MAPPINGS[..1]),
            unmap_call(1, 0, 0xC010_0000, 0x1000),
        ),
        (
            mapped(&GAME_OS_DMA_MAPPINGS[..1]),
            unmap_call(1, 0, 0xBFFF_F000, 0x1000),
        ),
        (
            mapped(&GAME_OS_DMA_MAPPINGS[..1]),
            unmap_call(1, 1, m.bus_address, m.length),
        ),
        (
            mapped(&GAME_OS_DMA_MAPPINGS[..1]),
            unmap_call(1, 0, 0xA000_8000, 0x1000),
        ),
        (
            mapped(&GAME_OS_DMA_MAPPINGS[..1]),
            unmap_call(1, 0, 0x9FFF_F000, 0x1000),
        ),
        (
            mapped(&GAME_OS_DMA_MAPPINGS[..1]),
            unmap_call(1, 1, 0xA000_0000, 0x1000),
        ),
    ];
    assert_eq!(cases.len(), 7);
    for (state, call) in cases {
        assert_eq!(
            unmap(&state, &call),
            answered(DMA_UNMAP_NO_MAPPING_R3),
            "{:?}",
            call.inputs
        );
    }
}

/// [OFW-4.93 lv1 a:0x22651C-0x226530] compares the mapping's first byte
/// before [OFW-4.93 lv1 a:0x226534-0x22653C] compares its length.
#[test]
fn an_address_past_the_mappings_first_byte_gets_r5_marked_before_the_length() {
    let m = network_mapping();
    let state = mapped(&GAME_OS_DMA_MAPPINGS[..1]);
    let cases = [
        unmap_call(1, 0, m.bus_address + 0x1000, m.length),
        unmap_call(1, 0, m.bus_address + 1, m.length),
        unmap_call(1, 0, m.bus_address + m.length - 1, 0x1000),
        unmap_call(1, 0, m.bus_address + 0x8_0000, 0),
    ];
    assert_eq!(cases.len(), 4);
    for call in cases {
        assert_eq!(
            unmap(&state, &call),
            answered(DMA_UNMAP_ADDRESS_R3),
            "{:?}",
            call.inputs
        );
    }
}

/// [OFW-4.93 lv1 a:0x226634-0x226640] answers -17 with r6 marked.
#[test]
fn a_length_other_than_the_mappings_gets_r6_marked() {
    let m = network_mapping();
    let state = mapped(&GAME_OS_DMA_MAPPINGS[..1]);
    let cases = [
        0,
        0x1000,
        m.length / 2,
        m.length + 0x1000,
        m.length * 2,
        u64::MAX,
    ];
    assert_eq!(cases.len(), 6);
    for length in cases {
        assert_eq!(
            unmap(&state, &unmap_call(1, 0, m.bus_address, length)),
            answered(DMA_UNMAP_LENGTH_R3),
            "{length:#x}"
        );
    }
}

#[test]
fn a_call_outside_the_read_paths_is_a_named_refusal() {
    let state = mapped(&GAME_OS_DMA_MAPPINGS[..1]);
    let m = network_mapping();
    let cases = [
        (unmap_call(2, 0, m.bus_address, m.length), OTHER_BUS.what),
        (unmap_call(3, 0, 0, 0), OTHER_BUS.what),
        (unmap_call(4, u64::MAX, 0, 0), OTHER_BUS.what),
        (unmap_call(1, 3, m.bus_address, m.length), OTHER_DEVICE.what),
        (unmap_call(1, u64::MAX, 0, 0), OTHER_DEVICE.what),
        (unmap_call(1, 0, 0xA000_0000, 0x1000), LV1_REGION.what),
        (unmap_call(1, 0, 0xA000_7FFF, 0x1000), LV1_REGION.what),
    ];
    assert_eq!(cases.len(), 7);
    for (call, what) in cases {
        assert_eq!(
            unmap(&state, &call),
            refused(&call, what),
            "{:?}",
            call.inputs
        );
    }
}

#[test]
fn an_unmap_at_a_firmware_with_no_read_handler_is_a_refusal() {
    let state = mapped(&GAME_OS_DMA_MAPPINGS[..1]);
    let call = unmap_of(network_mapping());
    for firmware in [None, Some("4.92"), Some("")] {
        assert_eq!(
            unmap_region(&call, firmware, &state),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_177_and_the_unmap_restores_the_sync_term() {
    assert!(answered_numbers().contains(&UNMAP_DEVICE_DMA_REGION));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = request(OPEN_DEVICE, MAP_SC, &[1, 0, 0]);
    assert_eq!(r3_of(&hv.dispatch(&open)), Some(0));
    let allocate = request(
        ALLOCATE_DEVICE_DMA_REGION,
        MAP_SC,
        &[1, 0, 0x10_0000, 20, 0],
    );
    assert_eq!(r3_of(&hv.dispatch(&allocate)), Some(0));
    let before = hv.sync_term();
    assert_eq!(r3_of(&hv.dispatch(&map_of(network_mapping()))), Some(0));
    assert_ne!(hv.sync_term(), before);
    let disposition = hv.dispatch(&unmap_of(network_mapping()));
    assert_eq!(r3_of(&disposition), Some(0));
    let outputs = match disposition {
        HypercallDisposition::Answered(answer) => answer.outputs(),
        HypercallDisposition::Refused(_) => None,
    };
    assert_eq!(outputs, Some([0; 7]));
    assert_eq!(hv.groups().device_bus().dma_mappings().count(), 0);
    assert_eq!(hv.groups().device_bus().regions_of(NETWORK).count(), 1);
    assert_eq!(hv.sync_term(), before);
}
