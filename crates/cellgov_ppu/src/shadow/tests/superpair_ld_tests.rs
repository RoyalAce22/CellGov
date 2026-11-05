//! Two `ld` loads from one base, the second 8 bytes past the first, fuse.

use crate::instruction::{FusedPair, PpuInstruction as I};
use crate::shadow::test_support::{build_from_words, ld_raw, lwz_raw};

#[test]
fn adjacent_same_base_lds_fuse() {
    let shadow = build_from_words(0, &[ld_raw(30, 1, 16), ld_raw(31, 1, 24)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LdLd {
            rt1: 30,
            rt2: 31,
            ra: 1,
            offset1: 16,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

/// [PPC-Book1 p:39 s:3.3.2] ld reads its base before it writes RT, so the
/// second load's EA comes from the first load's value when RT1 = RA.
#[test]
fn a_first_load_into_its_base_does_not_fuse() {
    let shadow = build_from_words(0, &[ld_raw(1, 1, 0), ld_raw(2, 1, 8)]);
    assert!(matches!(shadow.get(0), Some(I::Ld { .. })));
    assert!(matches!(shadow.get(4), Some(I::Ld { .. })));
}

/// [PPC-Book1 p:39 s:3.3.2] RA = 0 is the value 0, so a load into r0 does
/// not move the second EA.
#[test]
fn a_load_into_r0_from_the_zero_base_fuses() {
    let shadow = build_from_words(0, &[ld_raw(0, 0, 0x100), ld_raw(3, 0, 0x108)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LdLd {
            rt1: 0,
            rt2: 3,
            ra: 0,
            offset1: 0x100,
        }))
    );
}

#[test]
fn a_gap_an_overflow_or_another_base_does_not_fuse() {
    for (first, second) in [
        (ld_raw(3, 1, 0), ld_raw(4, 1, 16)),
        (ld_raw(3, 1, 32760), ld_raw(4, 1, -32768)),
        (ld_raw(3, 1, 0), ld_raw(4, 2, 8)),
        (ld_raw(3, 1, 8), ld_raw(4, 1, 0)),
    ] {
        let shadow = build_from_words(0, &[first, second]);
        assert!(
            matches!(shadow.get(0), Some(I::Ld { .. })),
            "{first:#010x} {second:#010x}"
        );
    }
}

#[test]
fn a_fused_ld_pair_disassembles_as_its_halves() {
    let pair = I::Fused(FusedPair::LdLd {
        rt1: 30,
        rt2: 31,
        ra: 1,
        offset1: -16,
    });
    let text = crate::instruction::AsmText {
        insn: &pair,
        addr: 0,
        symbols: None,
    }
    .to_string();
    assert_eq!(
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        "ld r30, -16(r1); ld r31, -8(r1)"
    );
}

#[test]
fn adjacent_same_base_lwzs_fuse_as_word_loads() {
    let shadow = build_from_words(0, &[lwz_raw(3, 1, 0), lwz_raw(4, 1, 4)]);
    assert!(matches!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LwzLwz { .. }))
    ));
}
