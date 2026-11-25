use super::*;
use crate::answer::HypercallRefusal;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::GAME_OS_MMIO_MAPPINGS;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007D8CC].
const KERNEL_SC: u64 = 0x8000_0000_0007_D8CC;

/// Device 1 of bus 1.
const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// The call with r3 to r7.
fn request(
    bus: u64,
    device: u64,
    bus_address: u64,
    size: u64,
    page_shift: u64,
) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&[bus, device, bus_address, size, page_shift]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: MAP_DEVICE_MMIO_REGION,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(36_735),
    }
}

/// The kernel's call, with the inputs of the console's record.
fn kernel_call() -> HypercallRequest {
    let row = GAME_OS_MMIO_MAPPINGS[0];
    request(
        row.bus,
        row.device,
        row.bus_address,
        row.size,
        row.page_shift,
    )
}

/// The mapping of the console's record of the kernel's call.
fn kernel_mapping() -> MmioMapping {
    let row = GAME_OS_MMIO_MAPPINGS[0];
    MmioMapping {
        bus: row.bus,
        device: row.device,
        bus_address: row.bus_address,
        size: row.size,
        lpar_address: row.lpar_address,
        page_shift: row.page_shift,
    }
}

/// The region counter value that the kernel's call takes in CellGov's
/// run of the 4.93 kernel.
const KERNEL_COUNTER: u64 = 7;

/// A memory state whose next region counter value is [`KERNEL_COUNTER`].
fn memory() -> MemoryState {
    let mut memory = MemoryState::default();
    memory.take_counter(KERNEL_COUNTER - 1);
    memory
}

/// The state with `devices` open.
fn opened(devices: &[OpenDevice]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &device in devices {
        state.apply(DeviceBusChange::Opened(device));
    }
    state
}

fn map(state: &DeviceBusState, request: &HypercallRequest) -> HypercallDisposition {
    map_mmio(
        request,
        Some("4.93"),
        state,
        &memory(),
        &HeldRegions::default(),
    )
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: MAP_DEVICE_MMIO_REGION,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

/// The answer to the kernel's call with the next region counter value
/// [`KERNEL_COUNTER`]: the captured address in r4, the mapping and the
/// counter value.
fn kernel_answer() -> HypercallDisposition {
    let mapping = kernel_mapping();
    let mut expected = HypercallAnswer::lv1_status(0, [mapping.lpar_address, 0, 0, 0, 0, 0, 0]);
    expected.push_change(DeviceBusChange::MmioRegionMapped(mapping));
    expected.push_change(MemoryChange::CounterTaken(KERNEL_COUNTER));
    HypercallDisposition::Answered(expected)
}

#[test]
fn the_kernel_call_gets_the_captured_lpar_address_in_r4_and_carries_the_mapping() {
    assert_eq!(map(&opened(&[DEVICE_1]), &kernel_call()), kernel_answer());
    assert_eq!(kernel_mapping().lpar_address, 0x4000_0007_0000);
}

#[test]
fn the_checks_before_the_map_function_answer_in_lv1s_order() {
    let open = opened(&[OpenDevice { bus: 1, device: 0 }, DEVICE_1]);
    let cases = [
        (
            request(5, 1, 0x301_0000, 0x1_0000, 12),
            MMIO_MAP_BUS_RANGE_R3,
        ),
        (request(u64::MAX, 0, 0, 0, 99), MMIO_MAP_BUS_RANGE_R3),
        (request(0, 1, 0x301_0000, 0x1_0000, 12), MMIO_MAP_NO_BUS_R3),
        (request(1, 0, 0x301_0000, 0, 99), MMIO_MAP_DEVICE_0_R3),
    ];
    assert_eq!(cases.len(), 4);
    for (call, r3) in cases {
        assert_eq!(map(&open, &call), answered(r3), "{:x?}", call.inputs);
    }
    let closed = DeviceBusState::default();
    for device in [0, 1, 2] {
        let call = request(1, device, 0x301_0000, 0x1_0000, 12);
        assert_eq!(
            map(&closed, &call),
            answered(MMIO_MAP_NOT_OPEN_R3),
            "{device}"
        );
    }
}

#[test]
fn the_device_map_checks_answer_in_lv1s_order_for_device_1() {
    let open = opened(&[DEVICE_1]);
    let cases = [
        (request(1, 1, 0x301_1000, 0x1000, 99), MMIO_MAP_NO_REGION_R3),
        (
            request(1, 1, 0x380_0000, 0x1_0000, 12),
            MMIO_MAP_NO_REGION_R3,
        ),
        (request(1, 1, 0x301_0000, 0, 99), MMIO_MAP_SIZE_R3),
        (request(1, 1, 0x301_0000, 0x1_0001, 12), MMIO_MAP_SIZE_R3),
        (request(1, 1, 0x2400, 0x400, 12), MMIO_MAP_SIZE_R3),
        (
            request(1, 1, 0x301_0000, 0x1_0000, 13),
            MMIO_MAP_PAGE_SHIFT_R3,
        ),
        (
            request(1, 1, 0x381_0000, 0x1000, 28),
            MMIO_MAP_PAGE_SHIFT_R3,
        ),
    ];
    assert_eq!(cases.len(), 7);
    for (call, r3) in cases {
        assert_eq!(map(&open, &call), answered(r3), "{:x?}", call.inputs);
    }
}

#[test]
fn another_bus_another_device_and_an_unread_firmware_are_refusals_that_name_them() {
    let open = opened(&[DEVICE_1, OpenDevice { bus: 1, device: 2 }]);
    for bus in [2, 3, OPEN_DEVICE_BUS_MAX] {
        let other_bus = request(bus, 0, 0x301_0000, 0x1_0000, 12);
        assert_eq!(
            map(&open, &other_bus),
            refused(&other_bus, OTHER_BUS.what),
            "{bus}"
        );
    }
    let other_device = request(1, 3, 0x301_0000, 0x1_0000, 12);
    assert_eq!(
        map(&open, &other_device),
        refused(&other_device, OTHER_DEVICE.what)
    );
    let call = kernel_call();
    for firmware in [None, Some("4.92"), Some("3.70")] {
        assert_eq!(
            map_mmio(&call, firmware, &open, &memory(), &HeldRegions::default()),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_size_and_page_shift_checks_answer_with_a_mapping_held_and_a_close_clears_the_mapping() {
    let mut state = opened(&[DEVICE_1]);
    state.apply(DeviceBusChange::MmioRegionMapped(kernel_mapping()));
    let call = kernel_call();
    assert_eq!(state.mmio_mappings().count(), 1);

    for (mapped_call, r3) in [
        (request(1, 1, 0x301_0000, 0, 12), MMIO_MAP_SIZE_R3),
        (request(1, 1, 0x301_0000, 0x1800, 12), MMIO_MAP_SIZE_R3),
        (
            request(1, 1, 0x301_0000, 0x1_0000, 13),
            MMIO_MAP_PAGE_SHIFT_R3,
        ),
    ] {
        assert_eq!(map(&state, &mapped_call), answered(r3), "{r3:x}");
    }
    assert_eq!(state.mmio_mapping(1, 1, 0x381_0000), None);
    assert_eq!(state.mmio_mapping(1, 2, 0x301_0000), None);
    assert_eq!(state.mmio_mapping(2, 1, 0x301_0000), None);

    state.apply(DeviceBusChange::Closed(OpenDevice { bus: 1, device: 0 }));
    assert_eq!(state.mmio_mapping(1, 1, 0x301_0000), Some(kernel_mapping()));
    state.apply(DeviceBusChange::Closed(DEVICE_1));
    assert_eq!(state.mmio_mappings().count(), 0);
    state.apply(DeviceBusChange::Opened(DEVICE_1));
    assert_eq!(map(&state, &call), kernel_answer());
}
