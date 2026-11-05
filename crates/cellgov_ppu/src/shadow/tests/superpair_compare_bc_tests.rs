//! `cmpdi`, its quickened zero form, `cmpd` and `cmplw` fuse with the next
//! `bc` only when that `bc` is relative and does not link. The fused
//! compare keeps the width and signedness of the original compare.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{FusedPair, PpuInstruction as I};
use crate::shadow::test_support::{bc_raw, build_from_words};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// Encodes a `cmpi` word (OPCD 11); `cmp` and `cmpl` are OPCD 31 with
/// XO 0 and XO 32.
fn cmpdi_raw(bf: u32, ra: u32, imm: i16) -> u32 {
    (11 << 26) | (bf << 23) | (1 << 21) | (ra << 16) | u32::from(imm as u16)
}

fn cmpd_raw(bf: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (bf << 23) | (1 << 21) | (ra << 16) | (rb << 11)
}

fn cmplw_raw(bf: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (bf << 23) | (ra << 16) | (rb << 11) | (32 << 1)
}

#[test]
fn cmpdi_and_its_zero_form_fuse_with_bc() {
    for imm in [0i16, -5] {
        let shadow = build_from_words(0, &[cmpdi_raw(1, 3, imm), bc_raw(0b01100, 6, 8)]);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::CmpdiBc {
                bf: 1,
                ra: 3,
                imm,
                bo: 0b01100,
                bi: 6,
                target_offset: 8,
            })),
            "imm {imm}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

#[test]
fn cmpd_and_cmplw_fuse_with_bc() {
    let cmpd = build_from_words(0, &[cmpd_raw(0, 3, 4), bc_raw(0b00100, 2, 12)]);
    assert_eq!(
        cmpd.get(0),
        Some(I::Fused(FusedPair::CmpdBc {
            bf: 0,
            ra: 3,
            rb: 4,
            bo: 0b00100,
            bi: 2,
            target_offset: 12,
        }))
    );
    let cmplw = build_from_words(0, &[cmplw_raw(7, 5, 6), bc_raw(0b01100, 28, -8)]);
    assert_eq!(
        cmplw.get(0),
        Some(I::Fused(FusedPair::CmplwBc {
            bf: 7,
            ra: 5,
            rb: 6,
            bo: 0b01100,
            bi: 28,
            target_offset: -8,
        }))
    );
}

#[test]
fn a_linking_or_absolute_bc_does_not_fuse() {
    let compares = [
        (
            cmpdi_raw(1, 3, -5),
            I::Cmpdi {
                bf: 1,
                ra: 3,
                imm: -5,
            },
        ),
        (cmpdi_raw(1, 3, 0), I::CmpdZero { bf: 1, ra: 3 }),
        (
            cmpd_raw(0, 3, 4),
            I::Cmpd {
                bf: 0,
                ra: 3,
                rb: 4,
            },
        ),
        (
            cmplw_raw(7, 5, 6),
            I::Cmplw {
                bf: 7,
                ra: 5,
                rb: 6,
            },
        ),
    ];
    // bc with LK = 1, then bc with AA = 1.
    for second in [bc_raw(0b01100, 2, 8) | 1, bc_raw(0b01100, 2, 8) | 2] {
        for (word, unfused) in compares {
            let shadow = build_from_words(0, &[word, second]);
            assert_eq!(shadow.get(0), Some(unfused), "{second:#010x}");
        }
    }
}

/// Runs `insn` at 0x1000 with r3 = `a` and r4 = `b`; returns the verdict and the new PC.
fn pc_after(insn: I, a: u64, b: u64) -> (ExecuteVerdict, u64) {
    let mut state = PpuState::new();
    state.pc = 0x1000;
    state.set_gpr(3, a);
    state.set_gpr(4, b);
    let verdict = execute(
        &insn,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    (verdict, state.pc)
}

/// [PPC-Book1 p:61 s:3.3.9] cmpl with L = 0 compares bits 32:63 of RA
/// and RB as unsigned: 0x8000_0000 is above 1, so blt cr0 (BO 01100,
/// BI 0) falls through. RB has the larger high word, so a signed or a
/// 64-bit compare gives LT and takes the branch.
#[test]
fn cmplw_bc_compares_the_low_words_unsigned() {
    let blt = I::Fused(FusedPair::CmplwBc {
        bf: 0,
        ra: 3,
        rb: 4,
        bo: 0b01100,
        bi: 0,
        target_offset: 16,
    });
    assert_eq!(
        pc_after(blt, 0x0000_0000_8000_0000, 0x0000_0001_0000_0001),
        (ExecuteVerdict::Continue, 0x1000)
    );
}

/// [PPC-Book1 p:60 s:3.3.9] cmp with L = 1 compares all 64 bits signed:
/// 0xFFFF_FFFF_0000_0000 is negative, so blt cr0 branches to the bc
/// slot (0x1004) plus 16.
#[test]
fn cmpd_bc_compares_all_64_bits_signed() {
    let blt = I::Fused(FusedPair::CmpdBc {
        bf: 0,
        ra: 3,
        rb: 4,
        bo: 0b01100,
        bi: 0,
        target_offset: 16,
    });
    assert_eq!(
        pc_after(blt, 0xFFFF_FFFF_0000_0000, 0),
        (ExecuteVerdict::Branch, 0x1014)
    );
}
