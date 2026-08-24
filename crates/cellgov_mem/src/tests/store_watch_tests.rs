//! Store-watch record layout, shared with the RPCS3-side hook.

use super::pack_record;

/// The layout is shared with the RPCS3-side hook, so a field order
/// or width change silently misreads every capture from both.
#[test]
fn a_record_packs_the_five_fields_at_their_documented_offsets() {
    let r = pack_record(
        0x0102_0304_0506_0708,
        0x1122_3344,
        0x99AA_BBCC,
        0x10,
        0xDEAD_BEEF,
    );
    assert_eq!(r.len(), 28);
    assert_eq!(&r[0..8], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&r[8..12], &0x1122_3344u32.to_le_bytes());
    assert_eq!(&r[12..16], &0x99AA_BBCCu32.to_le_bytes());
    assert_eq!(&r[16..20], &0x10u32.to_le_bytes());
    assert_eq!(&r[20..28], &0xDEAD_BEEFu64.to_le_bytes());
}

#[test]
fn a_record_keeps_the_low_32_bits_of_a_wide_effective_address() {
    let r = pack_record(0, 0, 0x0000_0001_9988_7766, 4, 0);
    assert_eq!(&r[12..16], &0x9988_7766u32.to_le_bytes());
}
