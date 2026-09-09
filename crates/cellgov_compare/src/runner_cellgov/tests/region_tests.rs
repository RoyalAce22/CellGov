//! Named-region extraction across address spaces; the extractor refuses a descriptor the run cannot read.

use super::*;
use cellgov_mem::{PageSize, Region, RegionAccess};

fn desc(name: &str, space: AddressSpaceId, addr: u64, size: u64) -> RegionDescriptor {
    RegionDescriptor {
        name: name.into(),
        space,
        addr,
        size,
    }
}

fn memory_with(bytes: &[u8]) -> GuestMemory {
    let mut mem = GuestMemory::new(bytes.len().max(1));
    let range = ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
    mem
}

fn boot_only(bytes: &[u8]) -> SpaceSnapshots {
    SpaceSnapshots::from([(AddressSpaceId::BOOT, memory_with(bytes))])
}

#[test]
fn extract_returns_named_region_within_bounds() {
    let spaces = boot_only(&[1u8, 2, 3, 4, 5, 6, 7, 8]);
    let extracted = extract_regions(&spaces, &[desc("head", AddressSpaceId::BOOT, 0, 4)]).unwrap();
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].name, "head");
    assert_eq!(extracted[0].addr, 0);
    assert_eq!(extracted[0].data, vec![1, 2, 3, 4]);
}

#[test]
fn an_out_of_bounds_addr_is_refused_naming_the_region() {
    let spaces = boot_only(&[0u8; 8]);
    let err = extract_regions(&spaces, &[desc("oob", AddressSpaceId::BOOT, 999_999, 16)])
        .expect_err("nothing is mapped at 999_999");
    match err {
        RegionExtractError::Unreadable {
            name,
            space,
            addr,
            size,
            source,
        } => {
            assert_eq!(name, "oob");
            assert_eq!(space, 0);
            assert_eq!(addr, 999_999);
            assert_eq!(size, 16);
            assert!(matches!(source, MemError::Unmapped(_)), "{source:?}");
        }
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[test]
fn a_range_running_past_the_end_of_memory_is_refused() {
    let spaces = boot_only(&[0xAA; 4]);
    let err = extract_regions(&spaces, &[desc("straddle", AddressSpaceId::BOOT, 2, 8)])
        .expect_err("the last 6 bytes are unmapped");
    assert!(
        matches!(&err, RegionExtractError::Unreadable { name, .. } if name == "straddle"),
        "{err:?}"
    );
}

#[test]
fn extract_returns_empty_when_no_regions_requested() {
    let spaces = boot_only(&[0u8; 32]);
    assert!(extract_regions(&spaces, &[]).unwrap().is_empty());
}

#[test]
fn a_child_space_region_reads_the_child_not_the_boot_space() {
    let child = AddressSpaceId::new(1);
    let spaces = SpaceSnapshots::from([
        (AddressSpaceId::BOOT, memory_with(&[0x11; 8])),
        (child, memory_with(&[0x22; 8])),
    ]);
    let extracted = extract_regions(
        &spaces,
        &[
            desc("boot", AddressSpaceId::BOOT, 0, 4),
            desc("child", child, 0, 4),
        ],
    )
    .unwrap();
    assert_eq!(extracted[0].data, vec![0x11; 4]);
    assert_eq!(extracted[1].data, vec![0x22; 4]);
}

#[test]
fn a_region_in_a_space_the_run_never_created_is_refused_listing_the_spaces_present() {
    let spaces = SpaceSnapshots::from([
        (AddressSpaceId::BOOT, memory_with(&[0x11; 8])),
        (AddressSpaceId::new(1), memory_with(&[0x22; 8])),
    ]);
    let err = extract_regions(&spaces, &[desc("ghost", AddressSpaceId::new(3), 0, 4)])
        .expect_err("space 3 was never created");
    assert_eq!(
        err,
        RegionExtractError::SpaceMissing {
            name: "ghost".into(),
            space: 3,
            present: vec![0, 1],
        }
    );
}

#[test]
fn the_first_refused_descriptor_in_declaration_order_is_the_one_reported() {
    let spaces = boot_only(&[0x11; 8]);
    let err = extract_regions(
        &spaces,
        &[
            desc("fine", AddressSpaceId::BOOT, 0, 4),
            desc("ghost", AddressSpaceId::new(3), 0, 4),
            desc("oob", AddressSpaceId::BOOT, 999_999, 4),
        ],
    )
    .expect_err("two descriptors read nothing");
    assert!(
        matches!(&err, RegionExtractError::SpaceMissing { name, .. } if name == "ghost"),
        "{err:?}"
    );
}

#[test]
fn an_auxiliary_region_above_the_flat_range_is_readable() {
    let mut mem = GuestMemory::new(0x100);
    mem.install_region(0xD000_0000, 0x1000, "stack", PageSize::Page4K)
        .unwrap();
    let range = ByteRange::new(GuestAddr::new(0xD000_0FF0), 4).unwrap();
    mem.apply_commit(range, &[9, 8, 7, 6]).unwrap();
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)]);
    let extracted = extract_regions(
        &spaces,
        &[desc("stack_tail", AddressSpaceId::BOOT, 0xD000_0FF0, 4)],
    )
    .unwrap();
    assert_eq!(extracted[0].data, vec![9, 8, 7, 6]);
}

#[test]
fn a_region_straddling_two_mapped_regions_is_refused() {
    let mut mem = GuestMemory::new(0x10);
    mem.install_region(0x10, 0x10, "aux", PageSize::Page4K)
        .unwrap();
    let low = ByteRange::new(GuestAddr::new(0), 0x10).unwrap();
    mem.apply_commit(low, &[0xAA; 0x10]).unwrap();
    let high = ByteRange::new(GuestAddr::new(0x10), 0x10).unwrap();
    mem.apply_commit(high, &[0xBB; 0x10]).unwrap();
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)]);
    let err = extract_regions(&spaces, &[desc("boundary", AddressSpaceId::BOOT, 0xC, 8)])
        .expect_err("no single region holds 0xC..0x14");
    assert!(
        matches!(
            &err,
            RegionExtractError::Unreadable {
                source: MemError::Unmapped(_),
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_region_whose_end_overflows_the_address_space_is_refused_as_overflow() {
    let spaces = boot_only(&[0x11; 8]);
    let err = extract_regions(
        &spaces,
        &[desc("wrap", AddressSpaceId::BOOT, u64::MAX - 1, 4)],
    )
    .expect_err("u64::MAX - 1 + 4 wraps");
    assert_eq!(
        err,
        RegionExtractError::Overflow {
            name: "wrap".into(),
            addr: u64::MAX - 1,
            size: 4,
        }
    );
}

#[test]
fn a_zero_length_region_inside_memory_yields_no_bytes() {
    let spaces = boot_only(&[0x11; 8]);
    let extracted =
        extract_regions(&spaces, &[desc("inside", AddressSpaceId::BOOT, 4, 0)]).unwrap();
    assert!(extracted[0].data.is_empty());
}

#[test]
fn the_exclusive_end_of_memory_admits_a_zero_length_region_but_not_one_byte() {
    // `Region::contains` (cellgov_mem guest.rs) admits `addr + length
    // <= end`, so the exclusive end holds an empty range and nothing
    // else.
    let spaces = boot_only(&[0x11; 8]);
    let empty = extract_regions(&spaces, &[desc("end", AddressSpaceId::BOOT, 8, 0)]).unwrap();
    assert!(empty[0].data.is_empty());
    let err = extract_regions(&spaces, &[desc("end", AddressSpaceId::BOOT, 8, 1)])
        .expect_err("byte 8 is the first byte past an 8-byte space");
    assert!(
        matches!(
            &err,
            RegionExtractError::Unreadable {
                addr: 8,
                size: 1,
                source: MemError::Unmapped(_),
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_zero_length_region_outside_memory_is_still_refused() {
    let spaces = boot_only(&[0x11; 8]);
    let err = extract_regions(
        &spaces,
        &[desc("outside", AddressSpaceId::BOOT, 999_999, 0)],
    )
    .expect_err("the address itself is unmapped");
    assert!(
        matches!(&err, RegionExtractError::Unreadable { name, .. } if name == "outside"),
        "{err:?}"
    );
}

#[test]
fn a_reserved_strict_region_is_refused_with_the_memory_layer_reason() {
    let mem = GuestMemory::from_regions(vec![Region::with_access(
        0x1000,
        0x10,
        "strict",
        PageSize::Page4K,
        RegionAccess::ReservedStrict,
    )])
    .unwrap();
    let spaces = SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)]);
    let err = extract_regions(&spaces, &[desc("strict", AddressSpaceId::BOOT, 0x1000, 8)])
        .expect_err("a strict region refuses reads");
    assert!(
        matches!(
            &err,
            RegionExtractError::Unreadable {
                source: MemError::ReservedStrictRead { .. },
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn every_refusal_names_the_region_in_its_message() {
    let spaces = boot_only(&[0x11; 8]);
    let cases = [
        desc("ghost", AddressSpaceId::new(3), 0, 4),
        desc("wrap", AddressSpaceId::BOOT, u64::MAX - 1, 4),
        desc("oob", AddressSpaceId::BOOT, 999_999, 4),
    ];
    for case in cases {
        let name = case.name.clone();
        let text = extract_regions(&spaces, &[case]).unwrap_err().to_string();
        assert!(text.contains(&format!("region {name} ")), "{text}");
    }
}
