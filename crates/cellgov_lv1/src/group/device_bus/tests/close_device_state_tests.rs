use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::{DmaMapping, DmaRegion, StorageBusOpen};
use crate::group::network::NetworkState;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_mem::lanes;
use cellgov_ps3_abi::lv1::call::{
    ALLOCATE_DEVICE_DMA_REGION, CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, CONSTRUCT_EVENT_RECEIVE_PORT,
    MAP_DEVICE_DMA_REGION, NET_SET_INTERRUPT_MASK, NET_SET_INTERRUPT_STATUS_INDICATOR, OPEN_DEVICE,
};
use cellgov_ps3_abi::lv1::device_bus::{GAME_OS_DMA_MAPPINGS, GAME_OS_DMA_REGIONS};
use cellgov_time::GuestTicks;

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// The region that the console's lv1 gave the 4.93 kernel's allocate
/// call, from the capture `lv1_device_dma_region`.
const REGION: DmaRegion = {
    let row = GAME_OS_DMA_REGIONS[0];
    DmaRegion {
        bus: row.bus,
        device: row.device,
        address: row.address,
        size: row.size,
        page_shift: row.page_shift,
    }
};

/// The mapping that the console's lv1 made for the 4.93 kernel's map
/// call, from the capture `lv1_device_dma_map`.
const MAPPING: DmaMapping = {
    let row = GAME_OS_DMA_MAPPINGS[0];
    DmaMapping {
        bus: row.bus,
        device: row.device,
        bus_address: row.bus_address,
        lpar_address: row.lpar_address,
        length: row.length,
    }
};

/// A region and a mapping of device 1, which no answer gives; the state
/// tests use them to show that a close keeps another device's entries.
const DEVICE_1_REGION: DmaRegion = DmaRegion {
    device: 1,
    address: 0xD000_0000,
    ..REGION
};
const DEVICE_1_MAPPING: DmaMapping = DmaMapping {
    device: 1,
    bus_address: 0xD000_0000,
    ..MAPPING
};

/// The flags that the kernel's map wrapper builds from a flags word of 0
/// [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc].
const KERNEL_FLAGS: u64 = 0xE000_0000_0000_0000;

/// The `sc` of the kernel's `construct_event_receive_port` that makes the
/// network device's port [OFW-4.93 lv2_kernel a:0x80000000000A28E0].
const KERNEL_PORT_SC: u64 = 0x8000_0000_000A_28E0;

/// The indicator address of the kernel's 191 call, as the 191 tests use.
const KERNEL_INDICATOR: u64 = 0x0087_B3D0;

/// The mask of the kernel's first 193 call
/// [OFW-4.93 lv2_kernel a:0x80000000000A5668-0x80000000000A5680].
const KERNEL_MASK: u64 = 0x1002_0000_0000_6000;

fn state_with(changes: &[DeviceBusChange]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for change in changes {
        state.apply(change.clone());
    }
    state
}

/// The close calls the device's method at +112 with the region's address
/// [OFW-4.93 lv1 a:0x2276C8-0x2276E4]. That path returns the range through
/// a method [OFW-4.93 lv1 a:0x22AC18-0x22AC3C] that passes it to a
/// function on a global table that CellGov does not hold
/// [OFW-4.93 lv1 a:0x22A5B4-0x22A62C]. So the state records the device's
/// pool as freed.
#[test]
fn a_close_removes_the_device_and_its_regions_and_mappings_records_its_pool_and_keeps_the_rest() {
    let others = [
        DeviceBusChange::Opened(DEVICE_1),
        DeviceBusChange::DmaRegionAllocated(DEVICE_1_REGION),
        DeviceBusChange::DmaRegionMapped(DEVICE_1_MAPPING),
        DeviceBusChange::StorageBusOpened(StorageBusOpen { r5: 0 }),
        DeviceBusChange::StorageBusPortConnected(20),
    ];
    let mut state = state_with(&others);
    let kept = state.clone();
    state.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    state.apply(DeviceBusChange::DmaRegionAllocated(REGION));
    state.apply(DeviceBusChange::DmaRegionMapped(MAPPING));
    assert_ne!(state.sync_term(), kept.sync_term());
    state.apply(DeviceBusChange::Closed(NETWORK_DEVICE));
    assert_ne!(state, kept);
    assert_eq!(state.freed_pools().collect::<Vec<_>>(), [NETWORK_DEVICE]);
    assert_eq!(
        state.sync_term(),
        kept.sync_term()
            .wrapping_add(lanes::value_term(185, 0, &NETWORK_DEVICE))
    );
    assert_eq!(state.open_devices().collect::<Vec<_>>(), [DEVICE_1]);
    assert_eq!(state.storage_bus_open(), kept.storage_bus_open());
    assert_eq!(state.storage_bus_port(), kept.storage_bus_port());
    assert_eq!(state.dma_regions().collect::<Vec<_>>(), [DEVICE_1_REGION]);
    assert_eq!(state.dma_mappings().collect::<Vec<_>>(), [DEVICE_1_MAPPING]);
    assert!(state.is_open(DEVICE_1));
}

#[test]
fn the_close_of_a_device_with_nothing_held_leaves_a_state_with_no_term() {
    let mut state = state_with(&[DeviceBusChange::Opened(DEVICE_1)]);
    assert_ne!(state.sync_term(), 0);
    state.apply(DeviceBusChange::Closed(DEVICE_1));
    assert_eq!(state, DeviceBusState::default());
    assert_eq!(state.sync_term(), 0);
}

/// [OFW-4.93 lv1 a:0x23E048-0x23E060] stores 0 in the mask and -1 in the
/// indicator; [OFW-4.93 lv1 a:0x23E0F8-0x23E108] stores 3 in the state
/// word. The reopen's setup stores only the state word 2 and +24
/// [OFW-4.93 lv1 a:0x23E1C8-0x23E1D4] [OFW-4.93 lv1 a:0x23E1F4].
#[test]
fn the_teardown_clears_indicator_and_port_stores_mask_0_and_state_word_3_and_setup_keeps_mask_0() {
    let mut network = NetworkState::default();
    network.apply(NetworkChange::SetUp);
    assert_eq!(network, NetworkState::default());
    network.apply(NetworkChange::StatusIndicatorSet(KERNEL_INDICATOR));
    network.apply(NetworkChange::InterruptPortConnected(19));
    network.apply(NetworkChange::InterruptMaskSet(KERNEL_MASK));
    network.apply(NetworkChange::TornDown);
    assert_eq!(network.status_indicator(), None);
    assert_eq!(network.interrupt_port(), None);
    assert_eq!(network.interrupt_mask(), Some(0));
    assert!(network.torn_down());
    let torn_down = network.clone();
    network.apply(NetworkChange::TornDown);
    assert_eq!(network, torn_down);

    network.apply(NetworkChange::SetUp);
    assert!(!network.torn_down());
    assert_eq!(network.interrupt_mask(), Some(0));
    assert_eq!(network.status_indicator(), None);
    let mut mask_0 = NetworkState::default();
    mask_0.apply(NetworkChange::InterruptMaskSet(0));
    assert_eq!(network, mask_0);
}

/// The call `number` with `registers` as r3 onward.
fn call(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: 0x8000_0000_000A_357C,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(40_000),
    }
}

fn r3_of(disposition: &HypercallDisposition) -> u64 {
    match disposition {
        HypercallDisposition::Answered(answer) => answer.r3,
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// Sends `number` with `registers` and asserts that the answer's r3 is 0.
fn dispatch_ok(hv: &mut Hypervisor, number: u64, registers: &[u64]) -> HypercallDisposition {
    let disposition = hv.dispatch(&call(number, registers));
    assert_eq!(r3_of(&disposition), 0, "hypercall {number}");
    disposition
}

/// Opens the network device and gives it the port, the indicator and the
/// mask.
fn open_network_device(hv: &mut Hypervisor, port: u64) {
    dispatch_ok(hv, OPEN_DEVICE, &[1, 0, 0]);
    dispatch_ok(hv, CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, &[1, 0, port, 0]);
    dispatch_ok(
        hv,
        NET_SET_INTERRUPT_STATUS_INDICATOR,
        &[1, 0, KERNEL_INDICATOR, 0],
    );
    dispatch_ok(hv, NET_SET_INTERRUPT_MASK, &[1, 0, KERNEL_MASK, 0]);
}

/// Opens the network device and gives it each value that the close
/// clears: the port, the indicator, the mask, the region and the mapping.
fn set_up_network_device(hv: &mut Hypervisor, port: u64) {
    open_network_device(hv, port);
    let allocate = dispatch_ok(hv, ALLOCATE_DEVICE_DMA_REGION, &[1, 0, REGION.size, 20, 0]);
    let HypercallDisposition::Answered(allocate) = allocate else {
        unreachable!("dispatch_ok returns an answer");
    };
    assert_eq!(
        allocate.outputs().map(|outputs| outputs[0]),
        Some(REGION.address)
    );
    dispatch_ok(
        hv,
        MAP_DEVICE_DMA_REGION,
        &[
            1,
            0,
            MAPPING.lpar_address,
            MAPPING.bus_address,
            MAPPING.length,
            KERNEL_FLAGS,
        ],
    );
}

/// Constructs an event receive port and returns its id.
fn construct_port(hv: &mut Hypervisor) -> u64 {
    let construct = HypercallRequest {
        pc: KERNEL_PORT_SC,
        ..call(CONSTRUCT_EVENT_RECEIVE_PORT, &[])
    };
    match hv.dispatch(&construct) {
        HypercallDisposition::Answered(made) => {
            assert_eq!(made.r3, 0, "construct_event_receive_port");
            made.outputs().expect("a table return")[0]
        }
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// [OFW-4.93 lv1 a:0x306F98] returns the close function's r3 and
/// [OFW-4.93 lv1 a:0x0C08] zeroes r4 to r10; a second close gets -11,
/// zero-extended [OFW-4.93 lv1 a:0x236534-0x236538]
/// [OFW-4.93 lv1 a:0x237520-0x23752C]; the teardown stores 0 in the mask
/// and 3 in the state word [OFW-4.93 lv1 a:0x23E048-0x23E060]
/// [OFW-4.93 lv1 a:0x23E0F8-0x23E108]. The close frees the region through
/// a method [OFW-4.93 lv1 a:0x22AC18-0x22AC3C] that passes the range to a
/// function on a global table that CellGov does not hold
/// [OFW-4.93 lv1 a:0x22A5B4-0x22A62C]. So the reopen's allocate is an
/// unmodelled input.
#[test]
fn the_hypervisor_close_tears_the_network_device_down_and_a_reopen_sets_it_up_without_a_region() {
    assert!(answered_numbers().contains(&CLOSE_DEVICE));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let port = construct_port(&mut hv);
    let before_open = hv.clone();
    set_up_network_device(&mut hv, port);
    assert_eq!(hv.groups().network().interrupt_port(), Some(port));
    assert_eq!(hv.groups().device_bus().dma_mappings().count(), 1);

    let closed = dispatch_ok(&mut hv, CLOSE_DEVICE, &[1, 0]);
    let HypercallDisposition::Answered(closed) = closed else {
        unreachable!("dispatch_ok returns an answer");
    };
    assert_eq!(closed.outputs(), Some([0; 7]));
    let devices = hv.groups().device_bus();
    assert_eq!(devices.open_devices().count(), 0);
    assert_eq!(devices.dma_regions().count(), 0);
    assert_eq!(devices.dma_mappings().count(), 0);
    assert_eq!(devices.freed_pools().collect::<Vec<_>>(), [NETWORK_DEVICE]);
    let mut torn_down = NetworkState::default();
    torn_down.apply(NetworkChange::TornDown);
    assert_eq!(*hv.groups().network(), torn_down);
    assert!(hv.groups().network().torn_down());
    assert_eq!(hv.groups().network().interrupt_mask(), Some(0));
    assert_ne!(hv, before_open);
    assert_ne!(hv.sync_term(), before_open.sync_term());

    let after_close = hv.clone();
    assert_eq!(
        r3_of(&hv.dispatch(&call(CLOSE_DEVICE, &[1, 0]))),
        0x0000_0000_FFFF_FFF5
    );
    assert_eq!(hv, after_close);

    open_network_device(&mut hv, port);
    let request = call(ALLOCATE_DEVICE_DMA_REGION, &[1, 0, REGION.size, 20, 0]);
    let allocate = hv.dispatch(&request);
    let HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number, unit, pc, ..
    }) = &allocate
    else {
        panic!("{allocate:?}");
    };
    assert_eq!(
        (number, unit, pc),
        (&ALLOCATE_DEVICE_DMA_REGION, &request.unit, &request.pc)
    );
    assert_eq!(hv.groups().device_bus().dma_regions().count(), 0);
    assert_eq!(
        hv.groups().network().status_indicator(),
        Some(KERNEL_INDICATOR)
    );
    assert_eq!(hv.groups().network().interrupt_mask(), Some(KERNEL_MASK));
    assert!(!hv.groups().network().torn_down());
}
