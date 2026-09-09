//! The extractor refuses a region whose bytes are provisional zeros.

use super::*;
use cellgov_mem::{PageSize, Region, RegionAccess};

fn desc(name: &str, addr: u64, size: u64) -> RegionDescriptor {
    RegionDescriptor {
        name: name.into(),
        space: AddressSpaceId::BOOT,
        addr,
        size,
    }
}

/// The shape of `main` beside `rsx` in a boot space.
///
/// The `0xAA` bytes at 0x10 make a user-region read distinguishable
/// from zeros.
fn boot_with_reserved_zero() -> SpaceSnapshots {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, 0x100, "main", PageSize::Page64K),
        Region::with_access(
            0x1000,
            0x100,
            "rsx",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
    ])
    .unwrap();
    let range = ByteRange::new(GuestAddr::new(0x10), 8).unwrap();
    mem.apply_commit(range, &[0xAA; 8]).unwrap();
    SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)])
}

#[test]
fn a_reserved_zero_readable_region_is_refused_naming_the_region_and_its_reserved_label() {
    let spaces = boot_with_reserved_zero();
    let err = extract_regions(&spaces, &[desc("window", 0x1010, 8)])
        .expect_err("0x1010 lies in the reserved-zero region");
    assert_eq!(
        err,
        RegionExtractError::Provisional {
            name: "window".into(),
            space: 0,
            addr: 0x1010,
            size: 8,
            region: "rsx",
        }
    );
}

#[test]
fn the_refusal_happens_before_any_read_so_no_provisional_read_is_counted() {
    let spaces = boot_with_reserved_zero();
    let _ = extract_regions(&spaces, &[desc("window", 0x1000, 0x100)]).unwrap_err();
    assert_eq!(spaces[&AddressSpaceId::BOOT].provisional_read_count(), 0);
}

#[test]
fn a_region_in_the_user_space_beside_a_reserved_region_is_still_read() {
    let spaces = boot_with_reserved_zero();
    let extracted = extract_regions(&spaces, &[desc("user", 0x10, 8)]).unwrap();
    assert_eq!(extracted[0].data, vec![0xAA; 8]);
    assert_eq!(spaces[&AddressSpaceId::BOOT].provisional_read_count(), 0);
}

/// The shape of `rsx` whose exclusive end is the base of the primary
/// `stack` in a boot space.
///
/// The `0xBB` bytes at the `stack` base make a user-region read
/// distinguishable from zeros.
fn reserved_zero_abutting_user() -> SpaceSnapshots {
    let mut mem = GuestMemory::from_regions(vec![
        Region::with_access(
            0x1000,
            0x100,
            "rsx",
            PageSize::Page4K,
            RegionAccess::ReservedZeroReadable,
        ),
        Region::new(0x1100, 0x100, "stack", PageSize::Page4K),
    ])
    .unwrap();
    let range = ByteRange::new(GuestAddr::new(0x1100), 8).unwrap();
    mem.apply_commit(range, &[0xBB; 8]).unwrap();
    SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)])
}

#[test]
fn a_range_ending_exactly_at_the_reserved_region_end_is_provisional() {
    let spaces = reserved_zero_abutting_user();
    let err = extract_regions(&spaces, &[desc("tail", 0x10F8, 8)]).unwrap_err();
    assert!(
        matches!(&err, RegionExtractError::Provisional { region: "rsx", .. }),
        "{err:?}"
    );
}

#[test]
fn a_range_at_the_user_base_right_after_the_reserved_region_is_read() {
    let spaces = reserved_zero_abutting_user();
    let extracted = extract_regions(&spaces, &[desc("stack_head", 0x1100, 8)]).unwrap();
    assert_eq!(extracted[0].data, vec![0xBB; 8]);
    assert_eq!(spaces[&AddressSpaceId::BOOT].provisional_read_count(), 0);
}

#[test]
fn a_range_crossing_out_of_the_reserved_region_is_unreadable_not_provisional() {
    let spaces = reserved_zero_abutting_user();
    let err = extract_regions(&spaces, &[desc("straddle", 0x10F8, 16)]).unwrap_err();
    assert!(
        matches!(
            &err,
            RegionExtractError::Unreadable {
                name,
                source: MemError::Unmapped(_),
                ..
            } if name == "straddle"
        ),
        "{err:?}"
    );
    assert_eq!(spaces[&AddressSpaceId::BOOT].provisional_read_count(), 0);
}

#[test]
fn a_zero_length_range_is_refused_as_empty_on_either_side_of_the_reserved_boundary() {
    let spaces = reserved_zero_abutting_user();
    for addr in [0x1010, 0x1100] {
        let err = extract_regions(&spaces, &[desc("empty", addr, 0)]).unwrap_err();
        assert!(
            matches!(&err, RegionExtractError::Empty { addr: a, .. } if *a == addr),
            "{err:?}"
        );
    }
    assert_eq!(spaces[&AddressSpaceId::BOOT].provisional_read_count(), 0);
}

#[test]
fn the_provisional_refusal_names_the_region_in_its_message() {
    let spaces = boot_with_reserved_zero();
    let text = extract_regions(&spaces, &[desc("window", 0x1000, 8)])
        .unwrap_err()
        .to_string();
    assert!(text.contains("region window "), "{text}");
    assert!(text.contains("reserved region rsx"), "{text}");
}
