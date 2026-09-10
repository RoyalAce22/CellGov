use super::{
    FIRMWARE_INTERNAL_PRX_STEMS, FLASH_MOUNT, FLASH_MOUNTS, GUEST_FLASH_MOUNT, SIBLING_FLASH_MOUNTS,
};

#[test]
fn internal_stems_carry_no_directory_or_extension() {
    // The loop below passes vacuously on an empty list, and a
    // firmware-exec boot that adds no internal module to its candidate
    // set fails later and somewhere else.
    assert_eq!(FIRMWARE_INTERNAL_PRX_STEMS.len(), 1);
    // The load site joins each stem with a directory and a .sprx/.prx
    // suffix. A stem that already carries either resolves to a path
    // that does not exist.
    for s in FIRMWARE_INTERNAL_PRX_STEMS {
        assert!(!s.is_empty(), "empty stem");
        assert!(
            !s.contains('/') && !s.contains('\\'),
            "{s:?} carries a directory separator"
        );
        assert!(
            !s.ends_with(".sprx") && !s.ends_with(".prx"),
            "{s:?} carries a file extension"
        );
    }
}

#[test]
fn every_flash_mount_name_is_one_bare_component() {
    for m in FLASH_MOUNTS {
        assert!(!m.is_empty(), "empty mount name");
        assert!(
            !m.contains('/') && !m.contains('\\'),
            "{m:?} carries a path separator"
        );
    }
}

#[test]
fn the_flash_mount_names_are_distinct() {
    // SIBLING_FLASH_MOUNTS indexes FLASH_MOUNTS, so a duplicated name
    // would make a sibling alias flash 1.
    let mut seen = std::collections::BTreeSet::new();
    for m in FLASH_MOUNTS {
        assert!(seen.insert(m), "{m:?} appears twice");
    }
}

#[test]
fn the_sibling_mounts_are_every_flash_mount_but_flash_one() {
    assert_eq!(SIBLING_FLASH_MOUNTS.as_slice(), &FLASH_MOUNTS[1..]);
    assert!(!SIBLING_FLASH_MOUNTS.contains(&FLASH_MOUNT));
}

#[test]
fn the_guest_flash_mount_is_flash_one_under_the_root() {
    assert_eq!(GUEST_FLASH_MOUNT, format!("/{FLASH_MOUNT}"));
}
