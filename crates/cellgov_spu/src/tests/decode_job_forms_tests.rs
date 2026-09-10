//! Decode of the forms a compiler-built job's crt0 and main use.
//!
//! Words with a register triple come from such a job's binary.

use super::*;

#[test]
fn hbra_from_binary() {
    assert_eq!(decode(0x1008_3c19).unwrap(), SpuInstruction::Hbra);
}

#[test]
fn dsync_decodes() {
    assert_eq!(decode(0x0060_0000).unwrap(), SpuInstruction::Dsync);
}

#[test]
fn cwx_from_binary() {
    assert_eq!(
        decode(0x3ac1_4207).unwrap(),
        SpuInstruction::Cwx {
            rt: 7,
            ra: 4,
            rb: 5
        }
    );
}

#[test]
fn cbx_from_binary() {
    assert_eq!(
        decode(0x3a82_8490).unwrap(),
        SpuInstruction::Cbx {
            rt: 16,
            ra: 9,
            rb: 10
        }
    );
}

#[test]
fn chx_and_cdx_decode() {
    assert_eq!(
        decode(0x3aa1_4207).unwrap(),
        SpuInstruction::Chx {
            rt: 7,
            ra: 4,
            rb: 5
        }
    );
    assert_eq!(
        decode(0x3ae1_4207).unwrap(),
        SpuInstruction::Cdx {
            rt: 7,
            ra: 4,
            rb: 5
        }
    );
}

#[test]
fn chd_from_binary() {
    assert_eq!(
        decode(0x3ea2_0086).unwrap(),
        SpuInstruction::Chd {
            rt: 6,
            ra: 1,
            imm: 8
        }
    );
}

#[test]
fn cdd_decodes() {
    assert_eq!(
        decode(0x3ee2_0086).unwrap(),
        SpuInstruction::Cdd {
            rt: 6,
            ra: 1,
            imm: 8
        }
    );
}

#[test]
fn rotqmbyi_from_binary() {
    assert_eq!(
        decode(0x3fbc_8204).unwrap(),
        SpuInstruction::Rotqmbyi {
            rt: 4,
            ra: 4,
            imm: 0x72
        }
    );
}

#[test]
fn ceqbi_from_binary_keeps_the_low_eight_immediate_bits() {
    assert_eq!(
        decode(0x7e00_0102).unwrap(),
        SpuInstruction::Ceqbi {
            rt: 2,
            ra: 2,
            imm: 0
        }
    );
    // I10 = 0x3AB: only 0xAB survives.
    assert_eq!(
        decode(0x7e00_0000 | (0x3AB << 14) | (9 << 7) | 3).unwrap(),
        SpuInstruction::Ceqbi {
            rt: 3,
            ra: 9,
            imm: 0xAB
        }
    );
}

#[test]
fn brhz_from_binary() {
    assert_eq!(
        decode(0x2200_0282).unwrap(),
        SpuInstruction::Brhz { rt: 2, offset: 5 }
    );
}

#[test]
fn biz_family_from_binary() {
    assert_eq!(
        decode(0x2500_004e).unwrap(),
        SpuInstruction::Biz { rt: 78, ra: 0 }
    );
    assert_eq!(
        decode(0x2520_004e).unwrap(),
        SpuInstruction::Binz { rt: 78, ra: 0 }
    );
    assert_eq!(
        decode(0x2540_004e).unwrap(),
        SpuInstruction::Bihz { rt: 78, ra: 0 }
    );
    assert_eq!(
        decode(0x2560_004e).unwrap(),
        SpuInstruction::Bihnz { rt: 78, ra: 0 }
    );
}

#[test]
fn rchcnt_from_binary() {
    assert_eq!(
        decode(0x01e0_0e82).unwrap(),
        SpuInstruction::Rchcnt { rt: 2, channel: 29 }
    );
}

#[test]
fn gb_and_gbh_are_gathers_not_hints() {
    assert_eq!(
        decode(0x3600_0283).unwrap(),
        SpuInstruction::Gb { rt: 3, ra: 5 }
    );
    assert_eq!(
        decode(0x3620_0283).unwrap(),
        SpuInstruction::Gbh { rt: 3, ra: 5 }
    );
}
