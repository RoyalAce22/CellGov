//! Byte-exact encodings for lis/ori/sc trampolines, blr, and packed OPD entries.

use super::*;

#[test]
fn encode_lis_ori_sc_for_callback_return_syscall_byte_pattern() {
    assert_eq!(
        encode_lis_ori_sc(0x80000),
        [
            0x3D, 0x60, 0x00, 0x08, // lis r11, 8       (hi = 8)
            0x61, 0x6B, 0x00, 0x00, // ori r11, r11, 0  (lo = 0)
            0x44, 0x00, 0x00, 0x02, // sc 0
        ],
    );
}

#[test]
fn encode_lis_ori_sc_for_hle_import_syscall_byte_pattern() {
    // 0x10005 -> hi = 1, lo = 5
    assert_eq!(
        encode_lis_ori_sc(0x10005),
        [
            0x3D, 0x60, 0x00, 0x01, // lis r11, 1
            0x61, 0x6B, 0x00, 0x05, // ori r11, r11, 5
            0x44, 0x00, 0x00, 0x02, // sc 0
        ],
    );
}

#[test]
fn encode_lis_ori_sc_low_half_only_byte_pattern() {
    // 0xFFFF -> hi = 0, lo = 0xFFFF
    assert_eq!(
        encode_lis_ori_sc(0xFFFF),
        [
            0x3D, 0x60, 0x00, 0x00, // lis r11, 0
            0x61, 0x6B, 0xFF, 0xFF, // ori r11, r11, 0xFFFF
            0x44, 0x00, 0x00, 0x02, // sc 0
        ],
    );
}

#[test]
fn encode_lis_ori_sc_high_half_only_byte_pattern() {
    // 0x1234_0000 -> hi = 0x1234, lo = 0
    assert_eq!(
        encode_lis_ori_sc(0x1234_0000),
        [
            0x3D, 0x60, 0x12, 0x34, // lis r11, 0x1234
            0x61, 0x6B, 0x00, 0x00, // ori r11, r11, 0
            0x44, 0x00, 0x00, 0x02, // sc 0
        ],
    );
}

#[test]
fn encode_lis_ori_sc_at_sign_safe_boundary_byte_pattern() {
    assert_eq!(
        encode_lis_ori_sc(0x7FFF_FFFF),
        [
            0x3D, 0x60, 0x7F, 0xFF, // lis r11, 0x7FFF
            0x61, 0x6B, 0xFF, 0xFF, // ori r11, r11, 0xFFFF
            0x44, 0x00, 0x00, 0x02, // sc 0
        ],
    );
}

#[test]
fn sc_byte_pattern_pins_mandatory_bit_30() {
    for syscall_num in [0u32, 1, 0x10000, 0x80000, 0x7FFF_FFFF] {
        let bytes = encode_lis_ori_sc(syscall_num);
        assert_eq!(bytes[8], 0x44, "sc opcode byte for {syscall_num:#x}");
        assert_eq!(bytes[9], 0x00, "sc reserved-zero byte for {syscall_num:#x}");
        assert_eq!(
            bytes[10], 0x00,
            "sc reserved-zero byte for {syscall_num:#x}"
        );
        assert_eq!(bytes[11], 0x02, "sc bit-30 byte for {syscall_num:#x}");
    }
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "would sign-extend")]
fn encode_lis_ori_sc_rejects_bit_31_set() {
    let _ = encode_lis_ori_sc(0x8000_0000);
}

#[test]
fn encode_blr_canonical_byte_pattern() {
    assert_eq!(encode_blr(), [0x4E, 0x80, 0x00, 0x20]);
}

#[test]
fn encode_ps3_packed_opd_byte_pattern() {
    assert_eq!(
        encode_ps3_packed_opd(0xCAFE_BABE, 0xDEAD_BEEF),
        [0xCA, 0xFE, 0xBA, 0xBE, 0xDE, 0xAD, 0xBE, 0xEF],
    );
}

#[test]
fn encode_ps3_packed_opd_zero_toc_byte_pattern() {
    assert_eq!(
        encode_ps3_packed_opd(0x0000_FF00, 0),
        [0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00],
    );
}
