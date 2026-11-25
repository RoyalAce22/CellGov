//! Device 2's MMIO maps of bus 1 in other call orders, and with inputs
//! that lv1's checks fail.

use super::*;
use crate::GroupStates;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

/// The group states with devices 1 and 2 of bus 1 open and the next
/// region counter value 7.
fn both_open() -> GroupStates {
    let mut groups = GroupStates::default();
    for device in [1, 2] {
        groups.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device }).into());
    }
    groups.apply(MemoryChange::CounterTaken(6).into());
    groups
}

/// The 4.93 answer to the call for device `device` of bus 1. The helper
/// applies the answer's changes to `groups`.
fn map_device(
    groups: &mut GroupStates,
    device: u64,
    bus_address: u64,
    size: u64,
    page_shift: u64,
) -> HypercallDisposition {
    let call = HypercallRequest {
        unit: UnitId::new(1),
        number: MAP_DEVICE_MMIO_REGION,
        inputs: [1, device, bus_address, size, page_shift, 0, 0, 0],
        pc: 0x8000_0000_0007_D8CC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(38_675),
    };
    let disposition = map_mmio(
        &call,
        Some("4.93"),
        groups.device_bus(),
        groups.memory().regions(),
        &HeldRegions::of(groups),
    );
    if let HypercallDisposition::Answered(answer) = &disposition {
        for change in answer.changes.clone() {
            groups.apply(change);
        }
    }
    disposition
}

/// The r3 and r4 of an answer, or `None` for a refusal.
fn r3_r4(disposition: &HypercallDisposition) -> Option<(u64, u64)> {
    match disposition {
        HypercallDisposition::Answered(answer) => {
            Some((answer.r3, answer.outputs().map_or(u64::MAX, |out| out[0])))
        }
        HypercallDisposition::Refused(_) => None,
    }
}

#[test]
fn device_2_maps_before_device_1_and_a_repeat_take_the_next_values_of_the_one_counter() {
    let mut groups = both_open();
    let calls = [
        (2, 0x302_0000, 0x4000_0007_0000),
        (2, 0x302_0000, 0x4000_0008_0000),
        (1, 0x301_0000, 0x4000_0009_0000),
        (2, 0x382_0000, 0x4000_000A_0000),
    ];
    for (device, bus_address, lpar_address) in calls {
        let answer = map_device(&mut groups, device, bus_address, 0x1_0000, 12);
        assert_eq!(
            r3_r4(&answer),
            Some((0, lpar_address)),
            "{device} {bus_address:#x}"
        );
    }
    assert_eq!(groups.memory().regions().next_counter(), Some(11));
    let mapped: Vec<(u64, u64, u64)> = groups
        .device_bus()
        .mmio_mappings()
        .map(|m| (m.device, m.bus_address, m.lpar_address))
        .collect();
    assert_eq!(
        mapped,
        [
            (1, 0x301_0000, 0x4000_0009_0000),
            (2, 0x302_0000, 0x4000_0007_0000),
            (2, 0x302_0000, 0x4000_0008_0000),
            (2, 0x382_0000, 0x4000_000A_0000)
        ]
    );

    // A close of device 2 removes its mappings and keeps device 1's.
    groups.apply(DeviceBusChange::Closed(OpenDevice { bus: 1, device: 2 }).into());
    let kept: Vec<(u64, u64)> = groups
        .device_bus()
        .mmio_mappings()
        .map(|m| (m.device, m.bus_address))
        .collect();
    assert_eq!(kept, [(1, 0x301_0000)]);
}

#[test]
fn a_device_2_call_that_fails_a_check_gets_lv1s_r3_and_takes_no_counter_value() {
    let mut groups = both_open();
    let cases = [
        // The address of a region of device 1.
        (0x301_0000, 0x1_0000, 12, MMIO_MAP_NO_REGION_R3),
        // An address inside a region of device 2, past its start.
        (0x302_1000, 0x1000, 12, MMIO_MAP_NO_REGION_R3),
        (0x382_0000, 0, 12, MMIO_MAP_SIZE_R3),
        (0x382_0000, 0x2_0000, 12, MMIO_MAP_SIZE_R3),
        // The 512-byte region in slot 0 holds no whole page.
        (0x2600, 0x200, 12, MMIO_MAP_SIZE_R3),
        (0x2600, 0x400, 12, MMIO_MAP_SIZE_R3),
        (0x302_0000, 0x1_0000, 13, MMIO_MAP_PAGE_SHIFT_R3),
        (0x302_0000, 0x1_0000, 24, MMIO_MAP_SIZE_R3),
    ];
    for (bus_address, size, page_shift, r3) in cases {
        assert_eq!(
            map_device(&mut groups, 2, bus_address, size, page_shift),
            lv1_answer(r3),
            "{bus_address:#x} {size:#x} {page_shift}"
        );
    }
    assert_eq!(groups.memory().regions().next_counter(), Some(7));
    assert_eq!(groups.device_bus().mmio_mappings().count(), 0);

    // The first call that passes the checks takes value 7.
    let answer = map_device(&mut groups, 2, 0x302_0000, 0x1_0000, 16);
    assert_eq!(r3_r4(&answer), Some((0, 0x4000_0007_0000)));
}
