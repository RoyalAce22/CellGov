use super::*;
use crate::{
    AddressSpace, AddressSpaceChange, DeviceBusChange, LogicalSpe, MemoryChange, MemoryRegion,
    MmioMapping, SpeChange,
};

/// 1 MB: e = 20, so the address holds 2^22 counter values.
const MB: u64 = 0x10_0000;

/// The 4.93 GameOS region counter at the kernel's first allocation.
const BOOT: u64 = 2;

fn held(addresses: &[u64]) -> HeldRegions {
    HeldRegions(addresses.iter().copied().collect())
}

fn address(size: u64, counter: u64) -> u64 {
    region_address(size, counter).expect("an address")
}

#[test]
fn a_counter_past_the_field_takes_the_wrapped_low_address() {
    let first = (1 << 22) + 5;
    assert_eq!(
        place(MB, first, BOOT, &held(&[])),
        Ok(Placement {
            address: (20 << 42) | (5 << 20),
            counter: first,
        })
    );
}

/// lv1 takes a further counter value when the insert finds an equal
/// address [OFW-4.93 lv1 a:0x2CEDE4-0x2CEDEC].
#[test]
fn a_wrapped_address_that_a_held_region_takes_is_skipped_to_the_next_value() {
    let first = (1 << 22) + 5;
    let taken = held(&[address(MB, 5), address(MB, 6), address(0x1000, 7)]);
    assert_eq!(
        place(MB, first, BOOT, &taken),
        Ok(Placement {
            address: address(MB, 7),
            counter: first + 2,
        })
    );
}

#[test]
fn a_wrapped_value_below_the_boot_value_is_refused() {
    for first in [1 << 22, (1 << 22) + 1, 3 << 22] {
        assert_eq!(
            place(MB, first, BOOT, &held(&[])),
            Err(NoPlacement::BootRegion),
            "{first:#x}"
        );
    }
    let taken = held(&[address(MB, (1 << 22) - 1)]);
    assert_eq!(
        place(MB, (1 << 22) - 1, BOOT, &taken),
        Err(NoPlacement::BootRegion)
    );
}

/// The handler tests build their expected refusal from `what`, so this
/// test holds each reason's text to the reason it names.
#[test]
fn each_reason_for_no_address_names_itself_in_its_refusal_text() {
    let cases = [
        (NoPlacement::TooLarge, "larger than 2^42 bytes"),
        (
            NoPlacement::BootRegion,
            "made before the kernel's first allocation",
        ),
        (NoPlacement::EveryAddressTaken, "every address"),
    ];
    assert_eq!(cases.len(), 3);
    for (reason, words) in cases {
        assert!(
            reason.what().contains(words),
            "{reason:?}: {}",
            reason.what()
        );
    }
}

/// lv1 fails after 2^(42 - e) attempts that each find a taken address
/// [OFW-4.93 lv1 a:0x2CEDE8] [OFW-4.93 lv1 a:0x2CEE08-0x2CEE24]. A
/// 2^41-byte region's address holds 2 counter values and a 2^42-byte
/// region's holds 1.
#[test]
fn a_size_whose_every_address_is_held_ends_with_every_address_taken() {
    let half = 1 << 41;
    let both = held(&[address(half, 0), address(half, 1)]);
    assert_eq!(
        place(half, 0, 0, &both),
        Err(NoPlacement::EveryAddressTaken)
    );
    assert_eq!(
        place(half, 7, 0, &both),
        Err(NoPlacement::EveryAddressTaken)
    );
    let one = held(&[address(half, 0)]);
    assert_eq!(
        place(half, 2, 0, &one),
        Ok(Placement {
            address: address(half, 1),
            counter: 3,
        })
    );
    let max = 1 << 42;
    assert_eq!(
        place(max, 9, 0, &held(&[42 << 42])),
        Err(NoPlacement::EveryAddressTaken)
    );
}

#[test]
fn a_run_of_held_addresses_ends_at_the_first_free_value_after_it() {
    let small = 0x1000;
    let first = (1 << 30) + 100;
    let run: Vec<u64> = (100..164).map(|value| address(small, value)).collect();
    assert_eq!(run.len(), 64);
    assert_eq!(
        place(small, first, BOOT, &held(&run)),
        Ok(Placement {
            address: address(small, 164),
            counter: first + 64,
        })
    );
}

#[test]
fn a_size_above_2_pow_42_is_refused() {
    assert_eq!(
        place((1 << 42) + 1, BOOT, BOOT, &held(&[])),
        Err(NoPlacement::TooLarge)
    );
}

#[test]
fn the_held_regions_are_each_counter_region_kind_of_the_partition() {
    let mut groups = GroupStates::default();
    let allocated = address(MB, 2);
    let table = address(0x4_0000, 3);
    let (area, shadow) = (address(0x8_0000, 4), address(0x1000, 5));
    let mmio = address(0x1_0000, 6);
    groups.apply(
        MemoryChange::Region(MemoryRegion {
            lpar_address: allocated,
            size: MB,
            page_size_exp: 20,
            unit_id: 1,
            counter: 2,
        })
        .into(),
    );
    groups.apply(
        AddressSpaceChange::Constructed(AddressSpace {
            vas_id: 2,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0x1014_0000_0000_0000,
            mapping: None,
        })
        .into(),
    );
    groups.apply(
        AddressSpaceChange::Mapped {
            vas_id: 2,
            lpar_address: table,
            counter: 3,
        }
        .into(),
    );
    groups.apply(
        SpeChange::Constructed(LogicalSpe {
            id: 0x10,
            spe_type: 1,
            vas_id: 2,
            area,
            shadow,
            shadow_real: 0,
        })
        .into(),
    );
    groups.apply(
        DeviceBusChange::MmioRegionMapped(MmioMapping {
            bus: 1,
            device: 1,
            bus_address: 0x301_0000,
            size: 0x1_0000,
            lpar_address: mmio,
            page_shift: 12,
        })
        .into(),
    );
    assert_eq!(
        HeldRegions::of(&groups),
        held(&[allocated, table, area, shadow, mmio])
    );
}
