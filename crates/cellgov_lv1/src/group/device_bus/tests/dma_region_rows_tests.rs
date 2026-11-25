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

/// The flags of the kernel's second device 1 call.
const FLAGS: u64 = 2;

/// Device 1's regions as (address, size, page shift), in the order of the
/// device's list and slots 0 to 4 in the capture `lv1_device_dma_slots`.
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
        tick: GuestTicks::new(38_419),
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

fn call(row: (u64, u64, u64)) -> HypercallRequest {
    request(1, row.1, row.2)
}

fn holding(devices: &[OpenDevice], regions: &[DmaRegion]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &device in devices {
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

#[test]
fn the_hypervisor_gives_device_1_its_five_captured_regions_in_order_and_refuses_a_sixth() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(1, 0, 0)
    };
    assert!(matches!(
        hv.dispatch(&open),
        HypercallDisposition::Answered(ref answer) if answer.r3 == 0
    ));
    assert_eq!(CAPTURED.len(), 5);
    for row in CAPTURED {
        let outputs = match hv.dispatch(&call(row)) {
            HypercallDisposition::Answered(answer) => answer.outputs(),
            HypercallDisposition::Refused(_) => None,
        };
        assert_eq!(
            outputs.map(|registers| registers[0]),
            Some(row.0),
            "{row:X?}"
        );
    }
    let held: Vec<_> = hv.groups().device_bus().dma_regions().collect();
    let expected: Vec<_> = CAPTURED.iter().map(|&row| region(1, row)).collect();
    assert_eq!(held, expected);

    let sixth = call(CAPTURED[0]);
    assert_eq!(hv.dispatch(&sixth), refused(&sixth, NEXT_REGION.what));
}

#[test]
fn each_later_device_1_call_after_the_earlier_rows_carries_its_captured_region() {
    assert_eq!(CAPTURED.len(), 5);
    for place in 1..CAPTURED.len() {
        let earlier: Vec<_> = CAPTURED[..place]
            .iter()
            .map(|&row| region(1, row))
            .collect();
        let state = holding(&[DEVICE_1], &earlier);
        assert_eq!(
            allocate(&state, &call(CAPTURED[place])),
            answered(region(1, CAPTURED[place])),
            "row {place}"
        );
    }
}

#[test]
fn a_device_1_call_out_of_the_captured_order_is_a_refusal() {
    let fresh = holding(&[DEVICE_1], &[]);
    let second = call(CAPTURED[1]);
    assert_eq!(
        allocate(&fresh, &second),
        refused(&second, OTHER_SHAPE.what)
    );
    let after_first = holding(&[DEVICE_1], &[region(1, CAPTURED[0])]);
    let third = call(CAPTURED[2]);
    assert_eq!(
        allocate(&after_first, &third),
        refused(&third, OTHER_SHAPE.what)
    );
}

#[test]
fn held_regions_other_than_the_first_captured_rows_are_a_refusal() {
    let moved = DmaRegion {
        address: 0xC010_0000,
        ..region(1, CAPTURED[0])
    };
    let cases = [
        vec![region(1, CAPTURED[1])],
        vec![moved],
        vec![region(1, CAPTURED[0]), region(1, CAPTURED[2])],
    ];
    assert_eq!(cases.len(), 3);
    for held in cases {
        let state = holding(&[DEVICE_1], &held);
        let next = call(CAPTURED[held.len()]);
        assert_eq!(
            allocate(&state, &next),
            refused(&next, OTHER_HELD.what),
            "{held:X?}"
        );
    }
}

/// lv1 searches six slots [OFW-4.93 lv1 a:0x23D54C-0x23D578] and returns
/// -2 when all are taken only after the pool gave a region
/// [OFW-4.93 lv1 a:0x23D544-0x23D590].
#[test]
fn a_device_1_or_2_call_after_six_held_regions_is_a_refusal() {
    assert_eq!(DMA_SLOT_COUNT, 6);
    for device in [1, 2] {
        let six: Vec<_> = (0..6)
            .map(|i| region(device, (0xF000_0000 + i * 0x1_0000, 0x1_0000, 12)))
            .collect();
        let state = holding(&[DEVICE_1, DEVICE_2], &six);
        let next = request(device, 0x1_0000, 12);
        assert_eq!(
            allocate(&state, &next),
            refused(&next, SLOTS_TAKEN.what),
            "device {device}"
        );
        let five = holding(&[DEVICE_1, DEVICE_2], &six[..5]);
        assert_ne!(
            allocate(&five, &next),
            refused(&next, SLOTS_TAKEN.what),
            "device {device}"
        );
    }
}
