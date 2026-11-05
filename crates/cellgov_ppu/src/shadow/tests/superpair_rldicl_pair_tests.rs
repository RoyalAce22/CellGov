//! Two adjacent `rldicl` fuse, raw or quickened to `clrldi` or `srdi`.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::encode::encode;
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// The word for `rldicl rA, rS, sh, mb` with Rc `rc`.
fn rldicl_raw(ra: u8, rs: u8, sh: u8, mb: u8, rc: bool) -> u32 {
    encode(&I::Rldicl { ra, rs, sh, mb, rc }).expect("rldicl encodes")
}

fn run(pair: FusedPair, state: &mut PpuState) -> ExecuteVerdict {
    execute(
        &I::Fused(pair),
        state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    )
}

/// Each of these pairs fuses:
/// - two raw rldicl, the second recording
/// - an srdi and a clrldi, both quickened by the shadow
/// - a recording rldicl and a quickened srdi
#[test]
fn two_rldicl_fuse_in_any_form() {
    let cases = [
        (
            (3u8, 4u8, 8u8, 4u8, false),
            (5u8, 3u8, 0u8, 32u8, true),
            0b10u8,
        ),
        ((3, 4, 60, 4, false), (3, 3, 0, 32, false), 0b00),
        ((3, 4, 0, 0, true), (6, 3, 56, 8, false), 0b01),
    ];
    for ((ra1, rs1, sh1, mb1, rc1), (ra2, rs2, sh2, mb2, rc2), rc) in cases {
        let words = [
            rldicl_raw(ra1, rs1, sh1, mb1, rc1),
            rldicl_raw(ra2, rs2, sh2, mb2, rc2),
        ];
        let shadow = build_from_words(0, &words);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::RldiclRldicl {
                ra1,
                rs1,
                sh1,
                mb1,
                ra2,
                rs2,
                sh2,
                mb2,
                rc,
            })),
            "{words:08x?}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

#[test]
fn an_rldicr_in_either_slot_does_not_fuse_as_an_rldicl_pair() {
    let rldicl = rldicl_raw(3, 4, 8, 4, false);
    let rldicr = encode(&I::Rldicr {
        ra: 3,
        rs: 3,
        sh: 8,
        me: 40,
        rc: false,
    })
    .expect("rldicr encodes");
    for words in [[rldicl, rldicr], [rldicr, rldicl]] {
        let shadow = build_from_words(0, &words);
        assert!(
            !matches!(
                shadow.get(0),
                Some(I::Fused(FusedPair::RldiclRldicl { .. }))
            ),
            "{words:08x?}"
        );
    }
}

/// [PPC-Book1 p:72 s:3.3.12] rldicl: RA <- ROTL64((RS), SH) & MASK(MB, 63).
/// [PPC-Book1 p:17 s:2.2] the second rldicl reads RS after the first wrote RA.
#[test]
fn the_second_rldicl_reads_what_the_first_wrote() {
    let mut state = PpuState::new();
    state.set_gpr(3, u64::MAX);
    state.set_gpr(4, 0x1234_5678_9ABC_DEF0);
    // srdi r3, r4, 4; clrldi. r5, r3, 32
    let verdict = run(
        FusedPair::RldiclRldicl {
            ra1: 3,
            rs1: 4,
            sh1: 60,
            mb1: 4,
            ra2: 5,
            rs2: 3,
            sh2: 0,
            mb2: 32,
            rc: 0b10,
        },
        &mut state,
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.gpr[3], 0x0123_4567_89AB_CDEF);
    assert_eq!(state.gpr[5], 0x0000_0000_89AB_CDEF);
    // [PPC-Book1 p:18 s:2.3.1] CR0 bits LT, GT, EQ from the 64-bit result.
    assert_eq!(
        [state.cr_bit(0), state.cr_bit(1), state.cr_bit(2)],
        [false, true, false]
    );
}

/// Bit 0 of `rc` records the first half's result, and bit 1 the second's.
#[test]
fn each_rc_bit_records_its_own_half() {
    for (rc, expected) in [
        (0b00u8, [false, false, false]),
        (0b01, [true, false, false]),
        (0b10, [false, false, true]),
        (0b11, [false, false, true]),
    ] {
        let mut state = PpuState::new();
        state.set_gpr(4, 0x8000_0000_0000_0001);
        // The first half copies a negative r4; the second keeps bit 63 of
        // r3 rotated left by 62, which is 0.
        let verdict = run(
            FusedPair::RldiclRldicl {
                ra1: 3,
                rs1: 4,
                sh1: 0,
                mb1: 0,
                ra2: 5,
                rs2: 3,
                sh2: 62,
                mb2: 63,
                rc,
            },
            &mut state,
        );
        assert_eq!(verdict, ExecuteVerdict::Continue);
        assert_eq!(state.gpr[3], 0x8000_0000_0000_0001);
        assert_eq!(state.gpr[5], 0);
        assert_eq!(
            [state.cr_bit(0), state.cr_bit(1), state.cr_bit(2)],
            expected,
            "rc {rc:#04b}"
        );
    }
}

#[test]
fn the_pair_declares_no_effects_and_continues() {
    let descriptor = I::Fused(FusedPair::RldiclRldicl {
        ra1: 3,
        rs1: 4,
        sh1: 8,
        mb1: 4,
        ra2: 5,
        rs2: 3,
        sh2: 0,
        mb2: 32,
        rc: 0,
    })
    .fuzz_descriptor(0);
    assert!(descriptor.effects.is_empty());
    assert_eq!(descriptor.outcomes, &[PpuOutcomeClass::Continue]);
}

#[test]
fn a_fused_rldicl_pair_disassembles_as_its_halves() {
    for (sh1, mb1, rc, text) in [
        (8u8, 4u8, 0b10u8, "rldicl r3, r4, 8, 4; clrldi. r5, r3, 32"),
        (60, 4, 0b01, "srdi. r3, r4, 4; clrldi r5, r3, 32"),
    ] {
        let pair = I::Fused(FusedPair::RldiclRldicl {
            ra1: 3,
            rs1: 4,
            sh1,
            mb1,
            ra2: 5,
            rs2: 3,
            sh2: 0,
            mb2: 32,
            rc,
        });
        let rendered = AsmText {
            insn: &pair,
            addr: 0,
            symbols: None,
        }
        .to_string();
        assert_eq!(
            rendered.split_whitespace().collect::<Vec<_>>().join(" "),
            text
        );
    }
}
