//! Tests for the quickening of `bc` and `bclr` into `Bdnz`, `BcTrue`,
//! `BcFalse` and `Blr`.

use super::quicken_insn;
use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{FusedPair, PpuInstruction as I};
use crate::shadow::test_support::{bc_raw, build_from_words, cmpw_raw, cmpwi_raw};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

fn bc(bo: u8) -> I {
    I::Bc {
        bo,
        bi: 6,
        offset: 8,
        aa: false,
        link: false,
    }
}

/// [PPC-Book1 p:20 s:2.4.1] BO 001at: branch if the CR bit is 0, for
/// every value of the "a" and "t" hint bits.
#[test]
fn every_001at_bc_is_bc_false_with_its_own_bo() {
    for bo in [0b00100, 0b00101, 0b00110, 0b00111] {
        assert_eq!(
            quicken_insn(bc(bo)),
            Some(I::BcFalse {
                bo,
                bi: 6,
                offset: 8
            }),
            "bo {bo:#07b}"
        );
    }
}

/// [PPC-Book1 p:20 s:2.4.1] BO 011at: branch if the CR bit is 1.
#[test]
fn every_011at_bc_is_bc_true_with_its_own_bo() {
    for bo in [0b01100, 0b01101, 0b01110, 0b01111] {
        assert_eq!(
            quicken_insn(bc(bo)),
            Some(I::BcTrue {
                bo,
                bi: 6,
                offset: 8
            }),
            "bo {bo:#07b}"
        );
    }
}

/// [PPC-Book1 p:20 s:2.4.1] BO 1a00t: decrement CTR, branch if CTR != 0.
#[test]
fn every_1a00t_bc_is_bdnz_with_its_own_bo() {
    for bo in [0b10000, 0b10001, 0b11000, 0b11001] {
        assert_eq!(
            quicken_insn(bc(bo)),
            Some(I::Bdnz {
                bo,
                bi: 6,
                offset: 8
            }),
            "bo {bo:#07b}"
        );
    }
}

/// [PPC-Book1 p:20 s:2.4.1] BO 1z1zz: branch always; the "z" bits are
/// ignored.
#[test]
fn every_1z1zz_bclr_is_blr_with_its_own_bo() {
    for bo in [
        0b10100, 0b10101, 0b10110, 0b10111, 0b11100, 0b11101, 0b11110, 0b11111,
    ] {
        let raw = I::Bclr {
            bo,
            bi: 3,
            link: false,
        };
        assert_eq!(
            quicken_insn(raw),
            Some(I::Blr { bo, bi: 3 }),
            "bo {bo:#07b}"
        );
    }
}

#[test]
fn bc_classes_that_test_ctr_and_a_cr_bit_or_bdz_or_always_stay_generic() {
    // 0000z, 0001z, 0100z, 0101z, 1a01t (bdz) and 1z1zz (always).
    for bo in [
        0b00000, 0b00010, 0b01000, 0b01010, 0b10010, 0b11011, 0b10100, 0b11111,
    ] {
        assert_eq!(quicken_insn(bc(bo)), None, "bo {bo:#07b}");
    }
}

#[test]
fn absolute_linking_conditional_lr_and_ctr_branches_stay_generic() {
    let absolute = I::Bc {
        bo: 0b01100,
        bi: 2,
        offset: 8,
        aa: true,
        link: false,
    };
    let linking = I::Bc {
        bo: 0b10000,
        bi: 0,
        offset: 8,
        aa: false,
        link: true,
    };
    let blrl = I::Bclr {
        bo: 0b10100,
        bi: 0,
        link: true,
    };
    let beqlr = I::Bclr {
        bo: 0b01100,
        bi: 2,
        link: false,
    };
    let bctr = I::Bcctr {
        bo: 0b10100,
        bi: 0,
        link: false,
    };
    for raw in [absolute, linking, blrl, beqlr, bctr] {
        assert_eq!(quicken_insn(raw), None, "{raw:?}");
    }
}

/// The verdict, PC and CTR after `insn` runs at PC 0x1000 with `ctr`.
fn run_with_ctr(insn: I, ctr: u64) -> (ExecuteVerdict, u64, u64) {
    let mut state = PpuState::new();
    state.pc = 0x1000;
    state.set_ctr(ctr);
    let verdict = execute(
        &insn,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    (verdict, state.pc, state.ctr())
}

/// [PPC-Book1 p:20 s:2.4.1] with BO bit 2 clear, the branch decrements
/// the whole 64-bit CTR, then tests it. CTR 1 falls through at 0, and
/// CTR 0 wraps to all ones and branches.
#[test]
fn bdnz_decrements_the_whole_ctr_before_testing_it() {
    let bdnz = I::Bdnz {
        bo: 0b10000,
        bi: 0,
        offset: 8,
    };
    assert_eq!(run_with_ctr(bdnz, 1), (ExecuteVerdict::Continue, 0x1000, 0));
    assert_eq!(
        run_with_ctr(bdnz, 0),
        (ExecuteVerdict::Branch, 0x1008, u64::MAX)
    );
}

#[test]
fn a_hinted_bne_after_cmpwi_still_fuses() {
    let shadow = build_from_words(0, &[cmpwi_raw(0, 3, 5), bc_raw(0b00111, 2, 8)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::CmpwiBc {
            bf: 0,
            ra: 3,
            imm: 5,
            bo: 0b00111,
            bi: 2,
            target_offset: 8,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

#[test]
fn a_hinted_bdnz_after_cmpwi_still_fuses() {
    let shadow = build_from_words(0, &[cmpwi_raw(1, 3, -1), bc_raw(0b11001, 0, -16)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::CmpwiBc {
            bf: 1,
            ra: 3,
            imm: -1,
            bo: 0b11001,
            bi: 0,
            target_offset: -16,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

#[test]
fn a_hinted_beq_after_cmpw_still_fuses() {
    let shadow = build_from_words(0, &[cmpw_raw(0, 3, 4), bc_raw(0b01111, 2, 12)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::CmpwBc {
            bf: 0,
            ra: 3,
            rb: 4,
            bo: 0b01111,
            bi: 2,
            target_offset: 12,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}
