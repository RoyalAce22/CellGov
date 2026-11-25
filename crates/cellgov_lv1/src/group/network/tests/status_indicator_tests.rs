use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::DeviceBusChange;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::network::{STATUS_INDICATOR_BUS_RANGE_R3, STATUS_INDICATOR_NO_DEVICE_R3};
use cellgov_ps3_abi::lv1::status::{LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's call
/// [OFW-4.93 lv2_kernel a:0x80000000000A34F4].
const KERNEL_SC: u64 = 0x8000_0000_000A_34F4;

/// The indicator address that the kernel passes in r5 in CellGov's
/// 4.93 boot: its device record's doubleword at +80, in the real mode
/// region.
const KERNEL_INDICATOR: u64 = 0x0087_B3D0;

/// An address in a region that `allocate_memory` creates: bits 42 to 47
/// hold a region size exponent.
const ALLOCATED_ADDRESS: u64 = (27 << 42) | 0x100;

/// The call for device `device` of bus `bus`, with `r5` and `r6`.
fn request(bus: u64, device: u64, r5: u64, r6: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[0] = bus;
    inputs[1] = device;
    inputs[2] = r5;
    inputs[3] = r6;
    HypercallRequest {
        unit: UnitId::new(0),
        number: NET_SET_INTERRUPT_STATUS_INDICATOR,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(32_835),
    }
}

/// The kernel's call: bus 1, device 0, its indicator and r6 = 0
/// [OFW-4.93 lv2_kernel a:0x80000000000A34E0-0x80000000000A34F0].
fn kernel_call() -> HypercallRequest {
    request(1, 0, KERNEL_INDICATOR, 0)
}

fn rm_size() -> u64 {
    real_mode_size("4.93").expect("the 4.93 real mode size")
}

fn opened() -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    state
}

fn indicator_set(address: u64) -> NetworkState {
    let mut state = NetworkState::default();
    state.apply(NetworkChange::StatusIndicatorSet(address));
    state
}

fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, [0; 7]))
}

fn refused(request: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: NET_SET_INTERRUPT_STATUS_INDICATOR,
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

/// [OFW-4.93 lv1 a:0x3084E0-0x3084FC] keeps the indicator;
/// [OFW-4.93 lv1 a:0x308588] returns 0.
#[test]
fn the_kernel_call_gets_r3_0_with_r4_to_r10_cleared_and_sets_the_indicator() {
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(NetworkChange::StatusIndicatorSet(KERNEL_INDICATOR));
    assert_eq!(
        set_status_indicator(
            &kernel_call(),
            Some("4.93"),
            &opened(),
            &NetworkState::default()
        ),
        HypercallDisposition::Answered(expected)
    );
}

/// [OFW-4.93 lv1 a:0x30848C-0x3084AC] returns -7 when +48 is not -1.
#[test]
fn a_call_while_an_indicator_is_set_gets_duplicate_entry_and_no_change() {
    for address in [KERNEL_INDICATOR, 0x10] {
        assert_eq!(
            set_status_indicator(
                &kernel_call(),
                Some("4.93"),
                &opened(),
                &indicator_set(address)
            ),
            answered(0x0000_0000_FFFF_FFF9),
            "set at {address:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x30829C-0x3082B0] checks the bus and the device before
/// the handler reads the device's state word.
#[test]
fn each_failed_device_check_gets_its_marked_status_before_the_open_check() {
    let cases = [
        (5, 0, STATUS_INDICATOR_BUS_RANGE_R3),
        (u64::MAX, 0, STATUS_INDICATOR_BUS_RANGE_R3),
        (0, 0, (1 << 32) | zero_extended(LV1_NO_ENTRY)),
        (2, 0, (1 << 32) | zero_extended(LV1_NO_ENTRY)),
        (1, 1, STATUS_INDICATOR_NO_DEVICE_R3),
    ];
    assert_eq!(cases.len(), 5);
    for (bus, device, r3) in cases {
        assert_eq!(
            set_status_indicator(
                &request(bus, device, KERNEL_INDICATOR, 0),
                Some("4.93"),
                &DeviceBusState::default(),
                &NetworkState::default()
            ),
            answered(r3),
            "bus {bus:#x} device {device:#x}"
        );
    }
}

#[test]
fn a_call_before_the_partition_opened_the_network_device_is_a_refusal() {
    let call = kernel_call();
    let mut other = DeviceBusState::default();
    other.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 1 }));
    for devices in [DeviceBusState::default(), other] {
        assert_eq!(
            set_status_indicator(&call, Some("4.93"), &devices, &NetworkState::default()),
            refused(&call, DEVICE_NOT_OPEN.what)
        );
    }
}

/// [OFW-4.93 lv1 a:0x3083CC-0x3083E0] checks r6 before
/// [OFW-4.93 lv1 a:0x3083FC-0x308414] checks the alignment.
#[test]
fn an_r6_other_than_0_gets_r6_marked_before_the_address_checks() {
    for r6 in [1, u64::MAX] {
        assert_eq!(
            set_status_indicator(
                &request(1, 0, KERNEL_INDICATOR + 1, r6),
                Some("4.93"),
                &opened(),
                &indicator_set(KERNEL_INDICATOR)
            ),
            answered((8 << 32) | zero_extended(LV1_ILLEGAL_PARAMETER_VALUE)),
            "r6 {r6:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x3083FC-0x308414] refuses an address with one of its
/// low 3 bits set;
/// [OFW-4.93 lv1 a:0x308450-0x308458] refuses an address that no region
/// holds. Both come before the indicator check.
#[test]
fn a_misaligned_or_regionless_address_gets_r5_marked_before_the_indicator_check() {
    let r5_marked = (4 << 32) | zero_extended(LV1_ILLEGAL_PARAMETER_VALUE);
    let cases = [
        KERNEL_INDICATOR + 1,
        KERNEL_INDICATOR + 4,
        rm_size(),
        rm_size() + 8,
    ];
    assert_eq!(cases.len(), 4);
    for address in cases {
        for network in [NetworkState::default(), indicator_set(KERNEL_INDICATOR)] {
            assert_eq!(
                set_status_indicator(
                    &request(1, 0, address, 0),
                    Some("4.93"),
                    &opened(),
                    &network
                ),
                answered(r5_marked),
                "address {address:#x}"
            );
        }
    }
}

#[test]
fn each_aligned_address_in_the_real_mode_region_sets_the_indicator() {
    let cases = [0, 8, rm_size() - 8];
    assert_eq!(cases.len(), 3);
    for address in cases {
        let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
        expected.push_change(NetworkChange::StatusIndicatorSet(address));
        assert_eq!(
            set_status_indicator(
                &request(1, 0, address, 0),
                Some("4.93"),
                &opened(),
                &NetworkState::default()
            ),
            HypercallDisposition::Answered(expected),
            "address {address:#x}"
        );
    }
}

#[test]
fn an_indicator_outside_the_real_mode_region_is_a_refusal() {
    let call = request(1, 0, ALLOCATED_ADDRESS, 0);
    assert_eq!(
        set_status_indicator(&call, Some("4.93"), &opened(), &NetworkState::default()),
        refused(&call, OUTSIDE_REAL_MODE.what)
    );
}

#[test]
fn a_call_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let call = kernel_call();
    for firmware in [None, Some("3.70"), Some("4.92")] {
        assert_eq!(
            set_status_indicator(&call, firmware, &opened(), &NetworkState::default()),
            refused(&call, UNREAD_FIRMWARE.what),
            "{firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_191_after_the_open_then_gets_duplicate_entry() {
    assert!(answered_numbers().contains(&NET_SET_INTERRUPT_STATUS_INDICATOR));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = request(1, 0, 0, 0);
    let open = HypercallRequest {
        number: cellgov_ps3_abi::lv1::call::OPEN_DEVICE,
        ..open
    };
    assert_eq!(r3_of(&hv.dispatch(&open)), Some(0));
    let opened_term = hv.sync_term();
    assert_eq!(r3_of(&hv.dispatch(&kernel_call())), Some(0));
    assert_eq!(
        hv.groups().network().status_indicator(),
        Some(KERNEL_INDICATOR)
    );
    assert_ne!(hv.sync_term(), opened_term);
    let after_first = hv.clone();
    assert_eq!(
        r3_of(&hv.dispatch(&kernel_call())),
        Some(0x0000_0000_FFFF_FFF9)
    );
    assert_eq!(hv, after_first);
}
