use super::super::dma_region::allocate_region;
use super::*;
use crate::answer::HypercallRefusal;
use crate::{answered_numbers, DmaMapping, DmaRegion, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{ALLOCATE_DEVICE_DMA_REGION, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::{
    GameOsDmaMapping, GameOsDmaRegion, GAME_OS_DMA_MAPPINGS, GAME_OS_DMA_REGIONS,
};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's free wrapper
/// [OFW-4.93 lv2_kernel a:0x8000000000095764].
const KERNEL_SC: u64 = 0x8000_0000_0009_5764;

const NETWORK: OpenDevice = OpenDevice { bus: 1, device: 0 };
const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };
const DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

fn request(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(41_002),
    }
}

/// The free call with r3 to r5.
fn free_call(bus: u64, device: u64, bus_address: u64) -> HypercallRequest {
    request(FREE_DEVICE_DMA_REGION, &[bus, device, bus_address])
}

/// The free call that names the address of `region`.
fn free_of(region: DmaRegion) -> HypercallRequest {
    free_call(region.bus, region.device, region.address)
}

fn region(row: &GameOsDmaRegion) -> DmaRegion {
    DmaRegion {
        bus: row.bus,
        device: row.device,
        address: row.address,
        size: row.size,
        page_shift: row.page_shift,
    }
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

/// The partition's region of the network device in the console's record.
fn network_region() -> DmaRegion {
    let row = GAME_OS_DMA_REGIONS
        .iter()
        .find(|row| row.device == 0)
        .expect("the network region");
    region(row)
}

/// The state with devices 0, 1 and 2 of bus 1 open, each captured region
/// held, and each mapping of `rows` made.
fn held(rows: &[GameOsDmaMapping]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in DMA_MAP_DEVICES {
        state.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device }));
    }
    for row in GAME_OS_DMA_REGIONS {
        state.apply(DeviceBusChange::DmaRegionAllocated(region(row)));
    }
    for row in rows {
        state.apply(DeviceBusChange::DmaRegionMapped(mapping(row)));
    }
    state
}

fn free(state: &DeviceBusState, call: &HypercallRequest) -> HypercallDisposition {
    free_region(call, Some("4.93"), state)
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn freed_answer(region: DmaRegion) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::DmaRegionFreed(region));
    HypercallDisposition::Answered(answer)
}

fn refused(call: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: FREE_DEVICE_DMA_REGION,
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

/// [OFW-4.93 lv1 a:0x2276C8-0x22773C] frees the region and returns 0;
/// the handler writes no return slot [OFW-4.93 lv1 a:0x306A38].
#[test]
fn a_free_of_the_unmapped_network_region_gets_0_and_carries_the_free() {
    let state = held(&[]);
    assert_eq!(
        free(&state, &free_of(network_region())),
        freed_answer(network_region())
    );
}

/// A close and a free reach the same free method
/// [OFW-4.93 lv1 a:0x227650], so the next allocate gets the freed-pool
/// refusal.
#[test]
fn after_the_free_the_region_is_gone_and_the_next_allocate_is_refused() {
    let mut state = held(&[]);
    state.apply(DeviceBusChange::DmaRegionFreed(network_region()));
    assert_eq!(state.regions_of(NETWORK).count(), 0);
    assert_eq!(state.freed_pools().collect::<Vec<_>>(), [NETWORK]);
    let again = request(
        ALLOCATE_DEVICE_DMA_REGION,
        &[1, 0, network_region().size, network_region().page_shift, 0],
    );
    let disposition = allocate_region(&again, Some("4.93"), &state);
    assert!(
        matches!(disposition, HypercallDisposition::Refused(_)),
        "{disposition:?}"
    );
}

/// [OFW-4.93 lv1 a:0x2276B8-0x2276C4] answers -9 for a region whose
/// mapping list at +72 is not empty.
#[test]
fn a_region_that_holds_a_mapping_gets_busy_with_r5_marked() {
    let network = held(&GAME_OS_DMA_MAPPINGS[..1]);
    assert_eq!(
        free(&network, &free_of(network_region())),
        answered(DMA_FREE_BUSY_R3)
    );
    let device_1_row = GAME_OS_DMA_MAPPINGS
        .iter()
        .find(|row| row.device == 1)
        .expect("a device 1 mapping");
    let state = held(std::slice::from_ref(device_1_row));
    let holder = state
        .regions_of(DEVICE_1)
        .find(|r| {
            device_1_row.bus_address >= r.address && device_1_row.bus_address - r.address < r.size
        })
        .expect("the region of the mapping");
    assert_eq!(free(&state, &free_of(holder)), answered(DMA_FREE_BUSY_R3));
}

#[test]
fn each_captured_region_with_no_mapping_frees_and_leaves_the_others() {
    let mut state = held(&[]);
    let regions: Vec<DmaRegion> = state.dma_regions().collect();
    assert_eq!(regions.len(), GAME_OS_DMA_REGIONS.len());
    for (freed, held_region) in regions.iter().enumerate() {
        assert_eq!(
            free(&state, &free_of(*held_region)),
            freed_answer(*held_region)
        );
        state.apply(DeviceBusChange::DmaRegionFreed(*held_region));
        assert_eq!(state.dma_regions().count(), regions.len() - freed - 1);
    }
    assert_eq!(
        state.freed_pools().collect::<Vec<_>>(),
        [NETWORK, DEVICE_1, DEVICE_2]
    );
}

#[test]
fn a_free_for_device_2_leaves_device_1s_region_at_the_same_address() {
    let mut state = held(&[]);
    let first = state
        .regions_of(DEVICE_2)
        .next()
        .expect("a device 2 region");
    let twin = state
        .regions_of(DEVICE_1)
        .find(|r| r.address == first.address)
        .expect("a device 1 region at the same address");
    assert_eq!(free(&state, &free_of(first)), freed_answer(first));
    state.apply(DeviceBusChange::DmaRegionFreed(first));
    assert!(state.regions_of(DEVICE_1).any(|r| r == twin));
    assert!(!state.pool_freed(DEVICE_1));
    assert!(state.pool_freed(DEVICE_2));
}

/// [OFW-4.93 lv1 a:0x3069A8-0x3069AC] checks the bus;
/// [OFW-4.93 lv1 a:0x2363E0] refuses bus 0 and then a device that the
/// partition did not open.
#[test]
fn each_handler_and_access_check_gets_its_status_before_the_region_lookup() {
    let none = DeviceBusState::default();
    let r = network_region();
    let cases = [
        (free_call(5, 0, r.address), DMA_FREE_BUS_RANGE_R3),
        (free_call(1 << 32, 0, 0), DMA_FREE_BUS_RANGE_R3),
        (free_call(u64::MAX, 7, 0), DMA_FREE_BUS_RANGE_R3),
        (free_call(0, 7, 0), DMA_FREE_NO_BUS_R3),
        (free_of(r), DMA_FREE_NOT_OPEN_R3),
        (free_call(1, 2, 0xC008_0000), DMA_FREE_NOT_OPEN_R3),
        (free_call(1, 0, 0xA000_0000), DMA_FREE_NOT_OPEN_R3),
    ];
    assert_eq!(cases.len(), 7);
    for (call, r3) in cases {
        assert_eq!(free(&none, &call), answered(r3), "{:?}", call.inputs);
    }
}

/// [OFW-4.93 lv1 a:0x2263BC-0x2263C8] answers -6 with r5 marked.
#[test]
fn a_bus_address_that_no_region_holds_gets_r5_marked() {
    let r = network_region();
    let empty = {
        let mut state = DeviceBusState::default();
        state.apply(DeviceBusChange::Opened(NETWORK));
        state
    };
    let state = held(&[]);
    let cases = [
        (&empty, free_of(r)),
        (&state, free_call(1, 0, r.address + r.size)),
        (&state, free_call(1, 0, r.address - 1)),
        (&state, free_call(1, 0, 0xA000_8000)),
        (&state, free_call(1, 0, 0x9FFF_F000)),
        (&state, free_call(1, 1, 0xA000_0000)),
    ];
    assert_eq!(cases.len(), 6);
    for (state, call) in cases {
        assert_eq!(
            free(state, &call),
            answered(DMA_FREE_NO_REGION_R3),
            "{:?}",
            call.inputs
        );
    }
}

/// [OFW-4.93 lv1 a:0x2262AC-0x2262B4] compares the region's address at
/// +48 before the free method checks the mappings.
#[test]
fn an_address_past_the_regions_first_byte_gets_r5_marked_before_busy() {
    let r = network_region();
    let cases = [
        (held(&[]), free_call(1, 0, r.address + 1)),
        (held(&[]), free_call(1, 0, r.address + r.size - 1)),
        (
            held(&GAME_OS_DMA_MAPPINGS[..1]),
            free_call(1, 0, r.address + 0x1000),
        ),
    ];
    assert_eq!(cases.len(), 3);
    for (state, call) in cases {
        assert_eq!(
            free(&state, &call),
            answered(DMA_FREE_ADDRESS_R3),
            "{:?}",
            call.inputs
        );
    }
}

#[test]
fn a_call_outside_the_read_paths_is_a_named_refusal() {
    let state = held(&[]);
    let r = network_region();
    let cases = [
        (free_call(2, 0, r.address), OTHER_BUS.what),
        (free_call(3, 0, 0), OTHER_BUS.what),
        (free_call(4, u64::MAX, 0), OTHER_BUS.what),
        (free_call(1, 3, r.address), OTHER_DEVICE.what),
        (free_call(1, u64::MAX, 0), OTHER_DEVICE.what),
        (free_call(1, 0, 0xA000_0000), LV1_REGION.what),
    ];
    assert_eq!(cases.len(), 6);
    for (call, what) in cases {
        assert_eq!(
            free(&state, &call),
            refused(&call, what),
            "{:?}",
            call.inputs
        );
    }
}

/// lv1's find checks no partition [OFW-4.93 lv1 a:0x22799C-0x2279C0], so
/// it returns lv1's own region, and the address check at +48 runs before
/// the free method reads that region's mappings
/// [OFW-4.93 lv1 a:0x2262AC-0x2262B4].
#[test]
fn an_address_past_the_first_byte_of_lv1s_own_region_gets_r5_marked() {
    let state = held(&[]);
    let cases = [
        free_call(1, 0, 0xA000_0001),
        free_call(1, 0, 0xA000_1000),
        free_call(1, 0, 0xA000_7FFF),
    ];
    assert_eq!(cases.len(), 3);
    for call in cases {
        assert_eq!(
            free(&state, &call),
            answered(DMA_FREE_ADDRESS_R3),
            "{:?}",
            call.inputs
        );
    }
}

/// The free method reads the mapping list at +72 of the region that the
/// device's own find returns [OFW-4.93 lv1 a:0x2276B8-0x2276C4], so a
/// mapping of device 2 does not make device 1's region at the same
/// address busy.
#[test]
fn a_mapping_of_another_device_at_the_same_address_leaves_the_region_free() {
    let device_2_row = GAME_OS_DMA_MAPPINGS
        .iter()
        .find(|row| row.device == 2 && row.bus_address == 0xD000_0000)
        .expect("the device 2 mapping at 0xD000_0000");
    let state = held(std::slice::from_ref(device_2_row));
    let device_1_region = state
        .regions_of(DEVICE_1)
        .find(|r| r.address == 0xD000_0000)
        .expect("the device 1 region at 0xD000_0000");
    let device_2_region = state
        .regions_of(DEVICE_2)
        .find(|r| r.address == 0xD000_0000)
        .expect("the device 2 region at 0xD000_0000");
    assert_eq!(
        free(&state, &free_of(device_1_region)),
        freed_answer(device_1_region)
    );
    assert_eq!(
        free(&state, &free_of(device_2_region)),
        answered(DMA_FREE_BUSY_R3)
    );
}

#[test]
fn a_free_at_a_firmware_with_no_read_handler_is_a_refusal() {
    let state = held(&[]);
    let call = free_of(network_region());
    for firmware in [None, Some("4.92"), Some("")] {
        assert_eq!(
            free_region(&call, firmware, &state),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_175_and_the_free_moves_the_sync_term_off_a_fresh_open() {
    assert!(answered_numbers().contains(&FREE_DEVICE_DMA_REGION));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = request(OPEN_DEVICE, &[1, 0, 0]);
    assert_eq!(r3_of(&hv.dispatch(&open)), Some(0));
    let opened = hv.sync_term();
    let allocate = request(ALLOCATE_DEVICE_DMA_REGION, &[1, 0, 0x10_0000, 20, 0]);
    assert_eq!(r3_of(&hv.dispatch(&allocate)), Some(0));
    let disposition = hv.dispatch(&free_of(network_region()));
    assert_eq!(r3_of(&disposition), Some(0));
    let outputs = match disposition {
        HypercallDisposition::Answered(answer) => answer.outputs(),
        HypercallDisposition::Refused(_) => None,
    };
    assert_eq!(outputs, Some([0; 7]));
    assert_eq!(hv.groups().device_bus().regions_of(NETWORK).count(), 0);
    assert!(hv.groups().device_bus().pool_freed(NETWORK));
    assert_ne!(hv.sync_term(), opened);
}
