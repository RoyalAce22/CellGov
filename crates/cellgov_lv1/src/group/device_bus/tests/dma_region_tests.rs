use super::*;
use crate::answer::HypercallRefusal;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_ps3_abi::lv1::device_bus::{
    DMA_REGION_CAPTURE, DMA_REGION_PAGE_SHIFT_R3, DMA_REGION_SIZE_R3, DMA_REGION_TOO_LARGE_R3,
};
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000009585C].
const KERNEL_SC: u64 = 0x8000_0000_0009_585C;

/// The size of the kernel's call: 1 MB.
const KERNEL_SIZE: u64 = 0x10_0000;

/// The page shift of the kernel's call: pages of 1 MB.
const KERNEL_PAGE_SHIFT: u64 = 20;

/// The number of regions in the network device's list in the capture.
const CAPTURED_REGIONS: usize = 2;

/// The value of each row of the capture's `dma_region.tsv`, as (field,
/// value). Both reads of each doubleword must agree.
fn captured_rows() -> Vec<(String, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DMA_REGION_CAPTURE)
        .join("peek/cech20-cex-493/dma_region.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 6, "{name}: {line}");
            assert_eq!(row[3..5], ["2", "yes"], "{name}: {line}");
            let digits = row[5]
                .strip_prefix("0x")
                .unwrap_or_else(|| panic!("{name}: {line}"));
            let value =
                u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{name}: {line}: {e}"));
            (row[1].to_string(), value)
        })
        .collect()
}

/// The bus address that the console capture `lv1_device_dma_region`
/// reads for the one region of the game OS partition with the size and
/// the page shift of the kernel's call.
fn kernel_address() -> u64 {
    let rows = captured_rows();
    let field = |n: usize, suffix: &str| {
        let name = format!("region_{n}_{suffix}");
        rows.iter()
            .find(|(row, _)| *row == name)
            .map(|&(_, value)| value)
            .unwrap_or_else(|| panic!("no row for {name}"))
    };
    let classes = rows
        .iter()
        .filter(|(row, _)| row.ends_with("_class"))
        .count();
    assert_eq!(classes, CAPTURED_REGIONS, "regions in the capture");
    let matching: Vec<u64> = (1..=CAPTURED_REGIONS)
        .filter(|&n| {
            field(n, "partition") == LPAR_ID_GAME_OS
                && field(n, "size") == KERNEL_SIZE
                && field(n, "page_shift") == KERNEL_PAGE_SHIFT
        })
        .map(|n| field(n, "address"))
        .collect();
    assert_eq!(matching.len(), 1, "game OS regions of the kernel's shape");
    matching[0]
}

fn request(bus: u64, device: u64, size: u64, page_shift: u64, flags: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..5].copy_from_slice(&[bus, device, size, page_shift, flags]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: ALLOCATE_DEVICE_DMA_REGION,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(32_912),
    }
}

/// The kernel's call: bus 1, device 0, 1 MB, pages of 1 MB and flags 0.
fn kernel_call() -> HypercallRequest {
    request(1, 0, KERNEL_SIZE, KERNEL_PAGE_SHIFT, 0)
}

/// The region that the kernel's call gets.
fn kernel_region() -> DmaRegion {
    DmaRegion {
        bus: 1,
        device: 0,
        address: kernel_address(),
        size: KERNEL_SIZE,
        page_shift: KERNEL_PAGE_SHIFT,
    }
}

/// The state with the network device open.
fn opened() -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    state
}

/// The state after the kernel's call.
fn allocated() -> DeviceBusState {
    let mut state = opened();
    state.apply(DeviceBusChange::DmaRegionAllocated(kernel_region()));
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

fn r3_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) => Some(answer.r3),
        HypercallDisposition::Refused(_) => None,
    }
}

/// [OFW-4.93 lv1 a:0x23707C-0x237088] stores the address in the slot of
/// r4 and returns 0.
#[test]
fn the_kernel_call_gets_the_captured_address_in_r4_and_carries_its_region() {
    let mut expected = HypercallAnswer::lv1_status(0, [kernel_address(), 0, 0, 0, 0, 0, 0]);
    expected.push_change(DeviceBusChange::DmaRegionAllocated(kernel_region()));
    assert_eq!(
        allocate(&opened(), &kernel_call()),
        HypercallDisposition::Answered(expected)
    );
}

/// [OFW-4.93 lv1 a:0x306B0C-0x306B18] checks the bus and then the flags;
/// [OFW-4.93 lv1 a:0x2363F0-0x236428] refuses bus 0 before any device
/// check.
#[test]
fn each_handler_check_gets_its_marked_status_before_the_open_check() {
    let state = DeviceBusState::default();
    let cases = [
        (request(5, 0, 0, 0, 4), DMA_REGION_BUS_RANGE_R3),
        (request(1 << 32, 0, 0, 0, 0), DMA_REGION_BUS_RANGE_R3),
        (request(1, 0, 0, 0, 4), DMA_REGION_FLAGS_RANGE_R3),
        (request(0, 7, 0, 0, 1 << 32), DMA_REGION_FLAGS_RANGE_R3),
        (request(0, 7, 0, 0, 0), DMA_REGION_NO_BUS_R3),
        (request(1, 0, 0, 0, 3), DMA_REGION_NOT_OPEN_R3),
        (kernel_call(), DMA_REGION_NOT_OPEN_R3),
    ];
    assert_eq!(cases.len(), 7);
    for (call, r3) in cases {
        assert_eq!(allocate(&state, &call), answered(r3), "{:?}", call.inputs);
    }
}

/// [OFW-4.93 lv1 a:0x237010-0x237024] checks the flags before the device's
/// allocate function reads the size.
#[test]
fn flags_from_1_to_3_get_r7_marked_before_the_size_checks() {
    let flags = [1, 2, 3];
    assert_eq!(flags.len(), 3);
    for flag in flags {
        assert_eq!(
            allocate(&opened(), &request(1, 0, 0, 13, flag)),
            answered(DMA_REGION_FLAGS_R3),
            "flags {flag}"
        );
    }
}

/// lv1 checks the size and the page shift at
/// [OFW-4.93 lv1 a:0x226848-0x22688C] and then at
/// [OFW-4.93 lv1 a:0x22D4B8-0x22D4D0].
#[test]
fn each_size_and_page_shift_check_gets_its_status_and_no_change() {
    let cases = [
        (request(1, 0, 0, 13, 0), DMA_REGION_SIZE_R3),
        (request(1, 0, KERNEL_SIZE, 13, 0), DMA_REGION_PAGE_SHIFT_R3),
        (request(1, 0, KERNEL_SIZE, 24, 0), DMA_REGION_SIZE_R3),
        (request(1, 0, 0x2_0000_0000, 24, 0), DMA_REGION_TOO_LARGE_R3),
    ];
    assert_eq!(cases.len(), 4);
    for state in [opened(), allocated()] {
        for (call, r3) in &cases {
            assert_eq!(allocate(&state, call), answered(*r3), "{:?}", call.inputs);
        }
    }
}

#[test]
fn a_second_region_of_the_device_is_a_refusal() {
    let call = kernel_call();
    assert_eq!(
        allocate(&allocated(), &call),
        refused(&call, NEXT_REGION.what)
    );
}

#[test]
fn a_size_or_page_shift_that_no_capture_gives_is_a_refusal() {
    let calls = [
        request(1, 0, 0x20_0000, KERNEL_PAGE_SHIFT, 0),
        request(1, 0, KERNEL_SIZE, 16, 0),
        request(1, 0, (1 << 63) | 0x100_0000, 24, 0),
    ];
    assert_eq!(calls.len(), 3);
    for call in calls {
        assert_eq!(
            allocate(&opened(), &call),
            refused(&call, OTHER_SHAPE.what),
            "{:?}",
            call.inputs
        );
    }
}

#[test]
fn a_call_for_another_bus_or_device_of_bus_1_is_a_refusal() {
    let cases = [
        (request(2, 0, KERNEL_SIZE, 20, 0), OTHER_BUS.what),
        (request(4, 0, KERNEL_SIZE, 20, 0), OTHER_BUS.what),
        (request(1, 3, KERNEL_SIZE, 20, 0), OTHER_DEVICE.what),
        (request(1, 1 << 32, KERNEL_SIZE, 20, 0), OTHER_DEVICE.what),
    ];
    assert_eq!(cases.len(), 4);
    for (call, what) in cases {
        assert_eq!(
            allocate(&opened(), &call),
            refused(&call, what),
            "{:?}",
            call.inputs
        );
    }
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let call = kernel_call();
    for firmware in [None, Some("4.92"), Some("3.70")] {
        assert_eq!(
            allocate_region(&call, firmware, &opened()),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_174_after_the_open_and_holds_the_region() {
    assert!(answered_numbers().contains(&ALLOCATE_DEVICE_DMA_REGION));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        ..request(1, 0, 0, 0, 0)
    };
    assert_eq!(r3_of(&hv.dispatch(&open)), Some(0));
    let before = hv.sync_term();
    let disposition = hv.dispatch(&kernel_call());
    assert_eq!(r3_of(&disposition), Some(0));
    let outputs = match disposition {
        HypercallDisposition::Answered(answer) => answer.outputs(),
        HypercallDisposition::Refused(_) => None,
    };
    assert_eq!(
        outputs.map(|registers| registers[0]),
        Some(kernel_address())
    );
    let regions: Vec<_> = hv.groups().device_bus().dma_regions().collect();
    assert_eq!(regions, [kernel_region()]);
    assert_ne!(hv.sync_term(), before);
}
