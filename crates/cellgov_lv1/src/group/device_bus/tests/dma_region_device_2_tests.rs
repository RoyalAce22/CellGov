use super::*;
use crate::answer::HypercallRefusal;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000009585C].
const KERNEL_SC: u64 = 0x8000_0000_0009_585C;

/// Device 1 of bus 1.
const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// Device 2 of bus 1.
const DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

/// Flags 1, the flags of the kernel's device 1 call. The slot path of
/// devices 1 and 2 takes the flags 0 to 3 to the same answer
/// [OFW-4.93 lv1 a:0x237108-0x23713C].
const FLAGS: u64 = 1;

/// Device 2's regions as (address, size, page shift), in the order of the
/// device's list and slots 0 to 4 in the capture `lv1_device_2_regions`.
const CAPTURED: [(u64, u64, u64); 5] = [
    (0xC000_0000, 0x4_0000, 16),
    (0xC008_0000, 0x8_0000, 16),
    (0xC080_0000, 0x80_0000, 16),
    (0xD000_0000, 0x80_0000, 20),
    (0xE000_0000, 0x1_0000, 12),
];

fn request(device: u64, size: u64, page_shift: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&[1, device, size, page_shift, FLAGS]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: ALLOCATE_DEVICE_DMA_REGION,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(40_466),
    }
}

fn region(device: u64, (address, size, page_shift): (u64, u64, u64)) -> DmaRegion {
    DmaRegion {
        bus: 1,
        device,
        address,
        size,
        page_shift,
    }
}

fn call(device: u64, row: (u64, u64, u64)) -> HypercallRequest {
    request(device, row.1, row.2)
}

fn holding(regions: &[DmaRegion]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in [DEVICE_1, DEVICE_2] {
        state.apply(DeviceBusChange::Opened(device));
    }
    for &region in regions {
        state.apply(DeviceBusChange::DmaRegionAllocated(region));
    }
    state
}

fn allocate(state: &DeviceBusState, request: &HypercallRequest) -> HypercallDisposition {
    allocate_region(request, Some("4.93"), state)
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: ALLOCATE_DEVICE_DMA_REGION,
        unit: request.unit,
        pc: request.pc,
        what,
    })
}

fn answered(region: DmaRegion) -> HypercallDisposition {
    let mut expected = HypercallAnswer::lv1_status(0, [region.address, 0, 0, 0, 0, 0, 0]);
    expected.push_change(DeviceBusChange::DmaRegionAllocated(region));
    HypercallDisposition::Answered(expected)
}

fn device_1_regions() -> Vec<DmaRegion> {
    CAPTURED.iter().map(|&row| region(1, row)).collect()
}

#[test]
fn the_hypervisor_gives_device_2_its_five_captured_regions_after_device_1s_and_refuses_a_sixth() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for device in [1, 2] {
        let open = HypercallRequest {
            number: OPEN_DEVICE,
            ..request(device, 0, 0)
        };
        assert!(matches!(
            hv.dispatch(&open),
            HypercallDisposition::Answered(ref answer) if answer.r3 == 0
        ));
    }
    assert_eq!(CAPTURED.len(), 5);
    for device in [1, 2] {
        for row in CAPTURED {
            let outputs = match hv.dispatch(&call(device, row)) {
                HypercallDisposition::Answered(answer) => answer.outputs(),
                HypercallDisposition::Refused(_) => None,
            };
            assert_eq!(
                outputs.map(|registers| registers[0]),
                Some(row.0),
                "device {device} {row:X?}"
            );
        }
    }
    let held: Vec<_> = hv.groups().device_bus().dma_regions().collect();
    let mut expected = device_1_regions();
    expected.extend(CAPTURED.iter().map(|&row| region(2, row)));
    assert_eq!(held, expected);

    let sixth = call(2, CAPTURED[0]);
    assert_eq!(hv.dispatch(&sixth), refused(&sixth, NEXT_REGION.what));
}

#[test]
fn each_device_2_call_after_its_earlier_rows_carries_its_captured_region() {
    assert_eq!(CAPTURED.len(), 5);
    for place in 0..CAPTURED.len() {
        let mut held = device_1_regions();
        held.extend(CAPTURED[..place].iter().map(|&row| region(2, row)));
        let state = holding(&held);
        assert_eq!(
            allocate(&state, &call(2, CAPTURED[place])),
            answered(region(2, CAPTURED[place])),
            "row {place}"
        );
    }
}

#[test]
fn a_device_2_call_out_of_the_captured_order_is_a_refusal() {
    let after_first = holding(&[region(2, CAPTURED[0])]);
    let third = call(2, CAPTURED[2]);
    assert_eq!(
        allocate(&after_first, &third),
        refused(&third, OTHER_SHAPE.what)
    );
    let skipped = holding(&[region(2, CAPTURED[1])]);
    let second = call(2, CAPTURED[1]);
    assert_eq!(
        allocate(&skipped, &second),
        refused(&second, OTHER_HELD.what)
    );
}
