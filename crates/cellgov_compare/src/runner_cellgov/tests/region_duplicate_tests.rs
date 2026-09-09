//! The extractor refuses a manifest that declares one region name twice.

use super::*;

fn desc(name: &str, space: AddressSpaceId, addr: u64, size: u64) -> RegionDescriptor {
    RegionDescriptor {
        name: name.into(),
        space,
        addr,
        size,
    }
}

fn boot_only(bytes: &[u8]) -> SpaceSnapshots {
    let mut mem = GuestMemory::new(bytes.len().max(1));
    let range = ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).unwrap();
    SpaceSnapshots::from([(AddressSpaceId::BOOT, mem)])
}

#[test]
fn a_repeated_region_name_is_refused_naming_it() {
    let spaces = boot_only(&[0x11; 16]);
    let err = extract_regions(
        &spaces,
        &[
            desc("x", AddressSpaceId::BOOT, 0, 4),
            desc("y", AddressSpaceId::BOOT, 4, 4),
            desc("x", AddressSpaceId::BOOT, 8, 4),
        ],
    )
    .expect_err("the third descriptor repeats the first's name");
    assert_eq!(err, RegionExtractError::Duplicate { name: "x".into() });
    assert!(err.to_string().starts_with("region x "), "{err}");
}

#[test]
fn a_repeated_name_is_refused_even_when_both_copies_are_identical() {
    let spaces = boot_only(&[0x11; 16]);
    let same = desc("x", AddressSpaceId::BOOT, 0, 4);
    let err = extract_regions(&spaces, &[same.clone(), same]).unwrap_err();
    assert!(
        matches!(&err, RegionExtractError::Duplicate { name } if name == "x"),
        "{err:?}"
    );
}

#[test]
fn a_repeated_name_across_two_spaces_is_still_one_name() {
    let child = AddressSpaceId::new(1);
    let spaces = SpaceSnapshots::from([
        (
            AddressSpaceId::BOOT,
            boot_only(&[0x11; 8]).remove(&AddressSpaceId::BOOT).unwrap(),
        ),
        (
            child,
            boot_only(&[0x22; 8]).remove(&AddressSpaceId::BOOT).unwrap(),
        ),
    ]);
    let err = extract_regions(
        &spaces,
        &[
            desc("x", AddressSpaceId::BOOT, 0, 4),
            desc("x", child, 0, 4),
        ],
    )
    .unwrap_err();
    assert!(
        matches!(&err, RegionExtractError::Duplicate { name } if name == "x"),
        "{err:?}"
    );
}

#[test]
fn a_repeated_name_is_refused_before_the_copy_is_read_or_sized() {
    let spaces = boot_only(&[0x11; 8]);
    // The second copy would be refused as empty, as unreadable, or as
    // naming an absent space on its own; the name wins.
    for copy in [
        desc("x", AddressSpaceId::BOOT, 0, 0),
        desc("x", AddressSpaceId::BOOT, 999_999, 4),
        desc("x", AddressSpaceId::new(3), 0, 4),
    ] {
        let err =
            extract_regions(&spaces, &[desc("x", AddressSpaceId::BOOT, 0, 4), copy]).unwrap_err();
        assert!(
            matches!(&err, RegionExtractError::Duplicate { name } if name == "x"),
            "{err:?}"
        );
    }
}

#[test]
fn an_earlier_refusal_still_wins_over_a_later_repeated_name() {
    let spaces = boot_only(&[0x11; 8]);
    let err = extract_regions(
        &spaces,
        &[
            desc("x", AddressSpaceId::BOOT, 0, 4),
            desc("ghost", AddressSpaceId::new(3), 0, 4),
            desc("x", AddressSpaceId::BOOT, 4, 4),
        ],
    )
    .unwrap_err();
    assert!(
        matches!(&err, RegionExtractError::SpaceMissing { name, .. } if name == "ghost"),
        "{err:?}"
    );
}

#[test]
fn distinct_names_extract_every_region() {
    let spaces = boot_only(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let extracted = extract_regions(
        &spaces,
        &[
            desc("x", AddressSpaceId::BOOT, 0, 4),
            desc("x2", AddressSpaceId::BOOT, 4, 4),
        ],
    )
    .unwrap();
    assert_eq!(extracted.len(), 2);
    assert_eq!(extracted[1].data, vec![5, 6, 7, 8]);
}
