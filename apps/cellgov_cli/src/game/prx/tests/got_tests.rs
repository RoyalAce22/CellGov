//! Unresolved-import trampoline encoding, including high-bit NID sign extension.

use super::*;

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[test]
fn trampoline_body_is_24_bytes() {
    let body = build_unresolved_trampoline_body(0x1234_5678);
    assert_eq!(body.len(), 24);
}

#[test]
fn trampoline_body_low_nid_encoding() {
    let body = build_unresolved_trampoline_body(0x3226_7A31);
    assert_eq!(read_u32(&body, 0), 0x3C80_3226); // lis  r4, 0x3226
    assert_eq!(read_u32(&body, 4), 0x6084_7A31); // ori  r4, r4, 0x7A31
    assert_eq!(read_u32(&body, 8), 0x7884_0020); // clrldi r4, r4, 32
    assert_eq!(read_u32(&body, 12), 0x3D60_0001); // lis  r11, 0x0001
    assert_eq!(read_u32(&body, 16), 0x4400_0002); // sc
    assert_eq!(read_u32(&body, 20), 0x4E80_0020); // blr
}

#[test]
fn trampoline_body_clears_sign_extension_for_high_bit_nid() {
    // NID 0x9D98AFA0 is cellSysutilRegisterCallback -- one of the
    // first real-world NIDs to hit the `hi >= 0x8000` sign-extension
    // path that the bare lis/ori pair gets wrong.
    let body = build_unresolved_trampoline_body(0x9D98_AFA0);
    assert_eq!(read_u32(&body, 0), 0x3C80_9D98);
    assert_eq!(read_u32(&body, 4), 0x6084_AFA0);
    assert_eq!(
        read_u32(&body, 8),
        0x7884_0020,
        "clrldi r4, r4, 32 must follow the lis/ori pair"
    );
}

#[test]
fn trampoline_slot_bytes_matches_layout() {
    assert_eq!(UNRESOLVED_TRAMP_SLOT_BYTES, 32);
}
