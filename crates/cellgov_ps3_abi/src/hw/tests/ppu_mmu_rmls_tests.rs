use super::*;

/// [CBE-Handbook p:336 s:11.2] Table 11-1, row by row.
const TABLE_11_1: [(u8, u64); 11] = [
    (0b0000, 256 << 30),
    (0b0001, 16 << 30),
    (0b0010, 1 << 30),
    (0b0011, 64 << 20),
    (0b0100, 256 << 20),
    (0b0101, 16 << 20),
    (0b0110, 8 << 20),
    (0b0111, 128 << 20),
    (0b1001, 4 << 20),
    (0b1010, 2 << 20),
    (0b1011, 1 << 20),
];

#[test]
fn each_rmls_value_selects_the_table_11_1_limit() {
    for (rmls, limit) in TABLE_11_1 {
        assert_eq!(rmls_limit(rmls), limit, "RMLS {rmls:04b}");
        assert_eq!(rmls_of_limit(limit), Some(rmls), "limit {limit:#x}");
    }
}

/// [CBE-Handbook p:336 s:11.2] Table 11-1 note 1: the PPE treats each reserved value as 1 MB.
#[test]
fn a_reserved_rmls_value_selects_1_mb_and_the_high_bits_are_ignored() {
    for rmls in [0b1000, 0b1100, 0b1101, 0b1110, 0b1111] {
        assert_eq!(rmls_limit(rmls), 1 << 20, "RMLS {rmls:04b}");
    }
    assert_eq!(rmls_limit(0xF0 | 0b0111), 128 << 20);
}

#[test]
fn a_limit_that_no_rmls_value_selects_has_no_rmls() {
    for limit in [
        0,
        1 << 12,
        512 << 20,
        2 << 30,
        4 << 30,
        512 << 30,
        1 << 62,
        0x0800_0001,
    ] {
        assert_eq!(rmls_of_limit(limit), None, "limit {limit:#x}");
    }
}

#[test]
fn the_largest_ppe_limit_is_the_one_rmls_0000_selects() {
    assert_eq!(rmls_limit(0b0000), PPE_RM_LIMIT_MAX);
    assert_eq!((0..=0xF).map(rmls_limit).max(), Some(PPE_RM_LIMIT_MAX));
}
