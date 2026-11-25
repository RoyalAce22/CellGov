use super::tests::{memory, request};
use super::*;
use crate::group::memory::allocator::NoPlacement;
use crate::group::state::{GroupStates, StateChange};
use crate::{AddressSpace, AddressSpaceChange, DeviceBusChange, MmioMapping};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::memory::region_address;

/// The group states when one device MMIO mapping of `size` bytes holds
/// `address`.
fn groups_with_mapping(address: u64, size: u64) -> GroupStates {
    let mut groups = GroupStates::default();
    groups.apply(
        DeviceBusChange::MmioRegionMapped(MmioMapping {
            bus: 1,
            device: 1,
            bus_address: 0x301_0000,
            size,
            lpar_address: address,
            page_shift: 12,
        })
        .into(),
    );
    groups
}

/// The held regions when one device MMIO mapping of `size` bytes holds
/// `address`.
fn held_mapping(address: u64, size: u64) -> HeldRegions {
    HeldRegions::of(&groups_with_mapping(address, size))
}

/// The counter values that the answer's changes record.
fn taken_counters(answer: &HypercallAnswer) -> Vec<u64> {
    answer
        .changes
        .iter()
        .filter_map(|change| match change {
            StateChange::Memory(change) => match **change {
                MemoryChange::CounterTaken(counter) => Some(counter),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The area's address holds 2^23 counter values and the shadow page's
/// 2^30. So from the counter value 2^30 + 5 the area takes the address
/// of value 5, and the shadow page's first address is that of value 6.
/// lv1 skips a taken address [OFW-4.93 lv1 a:0x2CEDE4-0x2CEDEC].
#[test]
fn a_shadow_page_whose_wrapped_address_a_held_region_takes_takes_the_next_value() {
    let next = (1 << 30) + 5;
    let taken = region_address(0x1000, 6).expect("a 4 KB address");
    let d = construct(
        &request(),
        Some("4.93"),
        &memory(Some(next)),
        &LogicalSpeState::default(),
        &held_mapping(taken, 0x1000),
    );
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    let outputs = answer.outputs().expect("r4 to r10");
    assert_eq!(
        outputs[2],
        region_address(0x8_0000, 5).expect("the area"),
        "r6"
    );
    assert_eq!(
        outputs[4],
        region_address(0x1000, 7).expect("the shadow"),
        "r8"
    );
    assert_eq!(taken_counters(&answer), [next + 2]);
}

/// From the counter value 2^23 + 5 the area's first address is that of
/// value 5. A held region takes it, so the area takes the value 2^23 + 6,
/// and the shadow page starts at the value after the area's last attempt
/// [OFW-4.93 lv1 a:0x2CED84-0x2CED94].
#[test]
fn a_held_area_address_moves_the_area_and_the_shadow_page_to_later_values() {
    let next = (1 << 23) + 5;
    let taken = region_address(0x8_0000, 5).expect("a 512 KB address");
    let d = construct(
        &request(),
        Some("4.93"),
        &memory(Some(next)),
        &LogicalSpeState::default(),
        &held_mapping(taken, 0x8_0000),
    );
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    let outputs = answer.outputs().expect("r4 to r10");
    assert_eq!(
        outputs[2],
        region_address(0x8_0000, 6).expect("the area"),
        "r6"
    );
    assert_eq!(
        outputs[4],
        region_address(0x1000, next + 2).expect("the shadow"),
        "r8"
    );
    assert_eq!(taken_counters(&answer), [next + 2]);
}

/// From the counter value 2^30 - 1 the area takes that value, and the
/// shadow page's value 2^30 wraps to 0, below the boot value. From
/// 2^30 - 2 the shadow page takes 2^30 - 1, the last value its address
/// holds.
#[test]
fn a_shadow_page_whose_counter_wraps_below_the_boot_value_is_refused() {
    let r = request();
    let construct_from = |next: u64| {
        construct(
            &r,
            Some("4.93"),
            &memory(Some(next)),
            &LogicalSpeState::default(),
            &HeldRegions::default(),
        )
    };
    assert_eq!(
        construct_from((1 << 30) - 1),
        refuse(&r, NoPlacement::BootRegion.what())
    );
    let d = construct_from((1 << 30) - 2);
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    assert_eq!(
        answer.outputs().expect("r4 to r10")[4],
        region_address(0x1000, (1 << 30) - 1).expect("the shadow"),
        "r8"
    );
    assert_eq!(taken_counters(&answer), [(1u64 << 30) - 1]);
}

/// From the counter value 5 the area takes value 5, and the shadow page's
/// first address is that of value 6. A device MMIO mapping in the group
/// states holds that address, so the shadow page takes value 7.
#[test]
fn the_dispatched_call_skips_a_shadow_page_address_that_a_mapping_in_the_group_states_holds() {
    let taken = region_address(0x1000, 6).expect("a 4 KB address");
    let mut groups = groups_with_mapping(taken, 0x1000);
    groups.apply(
        MemoryChange::AddressSpace(AddressSpaceChange::Constructed(AddressSpace {
            vas_id: request().r(8),
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0x1014_0000_0000_0000,
            mapping: None,
        }))
        .into(),
    );
    groups.apply(MemoryChange::CounterTaken(4).into());
    let d = crate::answer(
        &request(),
        crate::CallerTranslation::Loads,
        Some("4.93"),
        &groups,
        &GuestMemory::new(0),
    );
    let HypercallDisposition::Answered(answer) = d else {
        panic!("{d:?}");
    };
    let outputs = answer.outputs().expect("r4 to r10");
    assert_eq!(
        outputs[2],
        region_address(0x8_0000, 5).expect("the area"),
        "r6"
    );
    assert_eq!(
        outputs[4],
        region_address(0x1000, 7).expect("the shadow"),
        "r8"
    );
    assert_eq!(taken_counters(&answer), [7]);
}
