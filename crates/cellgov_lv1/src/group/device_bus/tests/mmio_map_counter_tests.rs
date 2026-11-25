//! The logical partition address of `map_device_mmio_region`: the address
//! of the region of the call's size at the next value of the partition's
//! region counter.

use super::*;
use crate::group::memory::allocator::NoPlacement;
use crate::{GroupStates, StateChange};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::GAME_OS_MMIO_MAPPINGS;
use cellgov_ps3_abi::lv1::memory::region_address;
use cellgov_time::GuestTicks;

/// Device 1 of bus 1.
const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// The call with r3 to r7, from the `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007D8CC].
fn request(bus_address: u64, size: u64, page_shift: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: MAP_DEVICE_MMIO_REGION,
        inputs: [1, 1, bus_address, size, page_shift, 0, 0, 0],
        pc: 0x8000_0000_0007_D8CC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(37_514),
    }
}

/// The group states with device 1 open and the next region counter value
/// `next_counter`, or no region yet for `None`.
fn groups(next_counter: Option<u64>) -> GroupStates {
    let mut groups = GroupStates::default();
    groups.apply(DeviceBusChange::Opened(DEVICE_1).into());
    if let Some(next) = next_counter {
        groups.apply(MemoryChange::CounterTaken(next - 1).into());
    }
    groups
}

fn map(groups: &GroupStates, request: &HypercallRequest) -> HypercallDisposition {
    map_mmio(
        request,
        Some("4.93"),
        groups.device_bus(),
        groups.memory().regions(),
        &HeldRegions::of(groups),
    )
}

/// The r3, the r4 and the changes of an answer.
fn answer_parts(d: HypercallDisposition) -> (u64, u64, Vec<StateChange>) {
    let HypercallDisposition::Answered(answer) = d else {
        panic!("not answered: {d:?}");
    };
    let r4 = answer.outputs().map_or(u64::MAX, |out| out[0]);
    (answer.r3, r4, answer.changes)
}

#[test]
fn the_kernels_two_calls_take_the_counter_values_7_and_8_and_get_the_captured_addresses() {
    let mut groups = groups(Some(7));
    let rows: Vec<_> = GAME_OS_MMIO_MAPPINGS
        .iter()
        .filter(|row| row.device == DEVICE_1.device)
        .collect();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let call = request(row.bus_address, row.size, row.page_shift);
        let (r3, r4, changes) = answer_parts(map(&groups, &call));
        assert_eq!((r3, r4), (0, row.lpar_address), "{:#x}", row.bus_address);
        for change in changes {
            groups.apply(change);
        }
    }
    assert_eq!(groups.memory().regions().next_counter(), Some(9));
    let mapped: Vec<(u64, u64, u64)> = groups
        .device_bus()
        .mmio_mappings()
        .map(|m| (m.bus_address, m.lpar_address, m.page_shift))
        .collect();
    assert_eq!(
        mapped,
        [
            (0x301_0000, 0x4000_0007_0000, 12),
            (0x381_0000, 0x4000_0008_0000, 12)
        ]
    );
}

#[test]
fn a_map_before_the_first_region_takes_the_boot_counter_value() {
    let (r3, r4, changes) = answer_parts(map(&groups(None), &request(0x381_0000, 0x1_0000, 12)));
    assert_eq!((r3, r4), (0, 0x4000_0002_0000));
    assert_eq!(region_address(0x1_0000, 2), Some(r4));
    assert!(changes.contains(&MemoryChange::CounterTaken(2).into()));
}

#[test]
fn a_size_that_is_not_a_multiple_of_the_page_size_gets_lv1s_size_r3() {
    let groups = groups(Some(7));
    let cases = [
        request(0x2400, 0x200, 12),
        request(0x301_0000, 0x8000, 16),
        request(0x381_0000, 0x1800, 12),
    ];
    assert_eq!(cases.len(), 3);
    for call in cases {
        assert_eq!(
            map(&groups, &call),
            lv1_answer(MMIO_MAP_SIZE_R3),
            "{:x?}",
            call.inputs
        );
    }
    let (r3, r4, _) = answer_parts(map(&groups, &request(0x301_0000, 0x8000, 12)));
    assert_eq!((r3, r4), (0, 0x3C00_0003_8000));
}

#[test]
fn a_second_map_of_a_mapped_region_takes_the_next_counter_value() {
    let mut groups = groups(Some(7));
    let call = request(0x301_0000, 0x1_0000, 12);
    let mut lpars = Vec::new();
    for _ in 0..2 {
        let (r3, r4, changes) = answer_parts(map(&groups, &call));
        assert_eq!(r3, 0);
        lpars.push(r4);
        for change in changes {
            groups.apply(change);
        }
    }
    assert_eq!(lpars, [0x4000_0007_0000, 0x4000_0008_0000]);
    assert_eq!(groups.device_bus().mmio_mappings().count(), 2);
    assert_eq!(groups.memory().regions().next_counter(), Some(9));
}

/// A 64 KB region's address holds 2^26 counter values, so the counter
/// value 2^26 wraps to the value 0, below the boot value.
#[test]
fn a_counter_value_that_wraps_below_the_boot_value_is_a_refusal() {
    let call = request(0x301_0000, 0x1_0000, 12);
    assert_eq!(
        map(&groups(Some(1 << 26)), &call),
        UnmodelledError::new(NoPlacement::BootRegion.what()).refusal(&call)
    );
    let (r3, _, _) = answer_parts(map(&groups(Some((1 << 26) - 1)), &call));
    assert_eq!(r3, 0);
}
