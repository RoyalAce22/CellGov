//! Two adjacent `ldu` loads fuse; an invalid `ldu`, or a second base the
//! first load writes, does not.

use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;

/// `ldu rT, off(rA)` (opcode 58, XO 1).
fn ldu_raw(rt: u32, ra: u32, off: i16) -> u32 {
    (58 << 26) | (rt << 21) | (ra << 16) | (u32::from(off as u16) & 0xFFFC) | 1
}

#[test]
fn two_ldus_on_one_base_fuse() {
    let shadow = build_from_words(0, &[ldu_raw(0, 9, 8), ldu_raw(11, 9, 8)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LduLdu {
            rt1: 0,
            ra1: 9,
            off1: 8,
            rt2: 11,
            ra2: 9,
            off2: 8,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

#[test]
fn two_ldus_on_two_bases_fuse() {
    let shadow = build_from_words(0, &[ldu_raw(3, 9, -8), ldu_raw(4, 10, 16)]);
    assert!(matches!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LduLdu { ra2: 10, .. }))
    ));
}

/// [PPC-Book1 p:39 s:3.3.2] an ldu with RA = 0 or RA = RT is invalid, and
/// a second base equal to RT1 reads the first load's value, so none of
/// these fuses.
#[test]
fn an_invalid_ldu_or_a_base_the_first_load_writes_does_not_fuse() {
    for (first, second) in [
        (ldu_raw(3, 0, 8), ldu_raw(4, 9, 8)),
        (ldu_raw(3, 3, 8), ldu_raw(4, 9, 8)),
        (ldu_raw(3, 9, 8), ldu_raw(4, 0, 8)),
        (ldu_raw(3, 9, 8), ldu_raw(4, 4, 8)),
        (ldu_raw(3, 9, 8), ldu_raw(4, 3, 8)),
    ] {
        let shadow = build_from_words(0, &[first, second]);
        assert!(
            !matches!(shadow.get(0), Some(I::Fused(_))),
            "{first:#010x} {second:#010x}"
        );
    }
}

#[test]
fn a_fused_ldu_pair_disassembles_as_its_halves() {
    let pair = I::Fused(FusedPair::LduLdu {
        rt1: 0,
        ra1: 9,
        off1: 8,
        rt2: 11,
        ra2: 10,
        off2: -16,
    });
    let text = AsmText {
        insn: &pair,
        addr: 0,
        symbols: None,
    }
    .to_string();
    assert_eq!(
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        "ldu r0, 8(r9); ldu r11, -16(r10)"
    );
}
