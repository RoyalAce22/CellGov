use super::*;
use crate::answer::HypercallRefusal;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_ps3_abi::lv1::device_bus::DMA_REGION_SIZE_R3;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000009585C].
const KERNEL_SC: u64 = 0x8000_0000_0009_585C;

/// Device 1 of bus 1.
const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// Device 2 of bus 1.
const DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

/// The size of the kernel's device 1 call: 256 KB.
const KERNEL_SIZE: u64 = 0x4_0000;

/// The page shift of the kernel's device 1 call: pages of 64 KB.
const KERNEL_PAGE_SHIFT: u64 = 16;

/// The flags of the kernel's device 1 call.
const KERNEL_FLAGS: u64 = 1;

/// The bus address of the device 1 region at the head of the device's
/// list in the capture `lv1_device_dma_slots`.
const CAPTURED_ADDRESS: u64 = 0xC000_0000;

fn request(bus: u64, device: u64, size: u64, page_shift: u64, flags: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&[bus, device, size, page_shift, flags]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: ALLOCATE_DEVICE_DMA_REGION,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(38_360),
    }
}

/// The kernel's device 1 call: bus 1, device 1, 256 KB, pages of 64 KB
/// and flags 1.
fn kernel_call() -> HypercallRequest {
    request(1, 1, KERNEL_SIZE, KERNEL_PAGE_SHIFT, KERNEL_FLAGS)
}

fn kernel_region() -> DmaRegion {
    DmaRegion {
        bus: 1,
        device: 1,
        address: CAPTURED_ADDRESS,
        size: KERNEL_SIZE,
        page_shift: KERNEL_PAGE_SHIFT,
    }
}

fn opened(devices: &[OpenDevice]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &device in devices {
        state.apply(DeviceBusChange::Opened(device));
    }
    state
}

fn allocate(state: &DeviceBusState, request: &HypercallRequest) -> HypercallDisposition {
    allocate_region(request, Some("4.93"), state)
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: ALLOCATE_DEVICE_DMA_REGION,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

fn kernel_answer() -> HypercallDisposition {
    let mut expected = HypercallAnswer::lv1_status(0, [CAPTURED_ADDRESS, 0, 0, 0, 0, 0, 0]);
    expected.push_change(DeviceBusChange::DmaRegionAllocated(kernel_region()));
    HypercallDisposition::Answered(expected)
}

/// The slot path joins device 0's store of the address
/// [OFW-4.93 lv1 a:0x237140-0x237144].
#[test]
fn the_kernels_device_1_call_gets_the_captured_address_and_carries_its_region() {
    assert_eq!(
        allocate(&opened(&[DEVICE_1]), &kernel_call()),
        kernel_answer()
    );
}

/// The slot path has no flags check [OFW-4.93 lv1 a:0x237108-0x23713C],
/// so the size check of the device's allocate function answers
/// [OFW-4.93 lv1 a:0x226848-0x22684C].
#[test]
fn devices_1_and_2_take_flags_from_1_to_3_to_the_size_checks() {
    let state = opened(&[DEVICE_1, DEVICE_2]);
    let flags = [1, 2, 3];
    assert_eq!(flags.len(), 3);
    for device in [1, 2] {
        for flag in flags {
            assert_eq!(
                allocate(&state, &request(1, device, 0, 13, flag)),
                answered(DMA_REGION_SIZE_R3),
                "device {device} flags {flag}"
            );
        }
    }
}

#[test]
fn a_device_1_or_2_call_before_its_open_gets_the_not_open_status() {
    let state = opened(&[NETWORK_DEVICE]);
    for device in [1, 2] {
        assert_eq!(
            allocate(
                &state,
                &request(1, device, KERNEL_SIZE, KERNEL_PAGE_SHIFT, 0)
            ),
            answered(DMA_REGION_NOT_OPEN_R3),
            "device {device}"
        );
    }
}

#[test]
fn a_second_device_1_region_is_a_refusal_and_a_network_region_is_not() {
    let network_region = DmaRegion {
        device: 0,
        ..kernel_region()
    };
    let mut state = opened(&[NETWORK_DEVICE, DEVICE_1]);
    state.apply(DeviceBusChange::DmaRegionAllocated(network_region));
    assert_eq!(allocate(&state, &kernel_call()), kernel_answer());

    state.apply(DeviceBusChange::DmaRegionAllocated(kernel_region()));
    let call = kernel_call();
    assert_eq!(allocate(&state, &call), refused(&call, NEXT_REGION.what));
}

#[test]
fn a_device_2_call_without_a_capture_and_a_device_3_call_are_refusals() {
    let state = opened(&[DEVICE_1, DEVICE_2]);
    let device_2 = request(1, 2, KERNEL_SIZE, KERNEL_PAGE_SHIFT, KERNEL_FLAGS);
    assert_eq!(
        allocate(&state, &device_2),
        refused(&device_2, OTHER_SHAPE.what)
    );
    let device_3 = request(1, 3, KERNEL_SIZE, KERNEL_PAGE_SHIFT, KERNEL_FLAGS);
    assert_eq!(
        allocate(&state, &device_3),
        refused(&device_3, OTHER_DEVICE.what)
    );
}

#[test]
fn the_hypervisor_answers_the_device_1_call_after_its_open_and_holds_the_region() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(1, 1, 0, 0, 0)
    };
    let r3 = match hv.dispatch(&open) {
        HypercallDisposition::Answered(answer) => Some(answer.r3),
        HypercallDisposition::Refused(_) => None,
    };
    assert_eq!(r3, Some(0));
    let before = hv.sync_term();
    let outputs = match hv.dispatch(&kernel_call()) {
        HypercallDisposition::Answered(answer) => answer.outputs(),
        HypercallDisposition::Refused(_) => None,
    };
    assert_eq!(
        outputs.map(|registers| registers[0]),
        Some(CAPTURED_ADDRESS)
    );
    let regions: Vec<_> = hv.groups().device_bus().dma_regions().collect();
    assert_eq!(regions, [kernel_region()]);
    assert_ne!(hv.sync_term(), before);
}
