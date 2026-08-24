//! Value-sample record layout.

use super::pack_record;

#[test]
fn a_record_packs_the_four_fields_at_their_documented_offsets() {
    let r = pack_record(0x0102_0304_0506_0708, 2, 3, &[0xAA, 0xBB, 0xCC, 0x00]);
    assert_eq!(r.len(), 13 + 4, "13-byte prefix then width value bytes");
    assert_eq!(&r[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(r[8], 2);
    assert_eq!(&r[9..13], &3u32.to_le_bytes());
    assert_eq!(&r[13..17], &[0xAA, 0xBB, 0xCC, 0x00]);
}

/// A short read pads the tail with zeroes; `actual_len` is what
/// keeps that padding distinguishable from a measured zero.
#[test]
fn a_short_read_record_still_carries_the_full_width_of_value_bytes() {
    let r = pack_record(7, 2, 1, &[0xFF, 0x00, 0x00, 0x00]);
    assert_eq!(r.len(), 17);
    assert_eq!(&r[9..13], &1u32.to_le_bytes());
    assert_eq!(&r[13..17], &[0xFF, 0x00, 0x00, 0x00]);
}
