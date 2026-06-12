//! IO-offset to effective-address translation window checks, identity and unrecorded maps.

use super::IoMap;

#[test]
fn identity_returns_offset_unchanged() {
    assert_eq!(IoMap::IDENTITY.translate(0), Some(0));
    assert_eq!(IoMap::IDENTITY.translate(0x1000), Some(0x1000));
    // `IDENTITY` uses size = u32::MAX, so u32::MAX trips the
    // in-window check (rel == size) and misses.
    assert_eq!(IoMap::IDENTITY.translate(u32::MAX), None);
    assert_eq!(IoMap::IDENTITY.translate(u32::MAX - 1), Some(u32::MAX - 1),);
}

#[test]
fn unrecorded_iomap_returns_none_matching_rpcs3_oracle() {
    let m = IoMap::default();
    assert_eq!(m.size, 0, "default IoMap means no iomap recorded");
    assert_eq!(m.translate(0), None);
    assert_eq!(m.translate(0x1000), None);
    assert_eq!(m.translate(u32::MAX), None);
}

#[test]
fn translates_offset_into_ea_inside_mapped_range() {
    let m = IoMap {
        ea: 0x4000_0000,
        io: 0x1000,
        size: 0x1000,
    };
    assert_eq!(m.translate(0x1000), Some(0x4000_0000));
    assert_eq!(m.translate(0x1234), Some(0x4000_0234));
    assert_eq!(m.translate(0x1FFF), Some(0x4000_0FFF));
}

#[test]
fn rejects_offset_below_io_base() {
    let m = IoMap {
        ea: 0x4000_0000,
        io: 0x1000,
        size: 0x1000,
    };
    assert_eq!(m.translate(0x0FFF), None);
    assert_eq!(m.translate(0), None);
}

#[test]
fn rejects_offset_at_or_beyond_size() {
    let m = IoMap {
        ea: 0x4000_0000,
        io: 0x1000,
        size: 0x1000,
    };
    assert_eq!(m.translate(0x2000), None);
    assert_eq!(m.translate(0xFFFF_FFFF), None);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "malformed IoMap")]
fn in_window_ea_overflow_panics_in_debug() {
    let m = IoMap {
        ea: 0xFFFF_F000,
        io: 0,
        size: 0x2000,
    };
    let _ = m.translate(0x1500);
}
