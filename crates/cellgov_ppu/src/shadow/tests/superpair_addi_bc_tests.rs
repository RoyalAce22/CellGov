//! `addi` and its quickened `li` fuse with the next `bc`, quickened or not,
//! only when that `bc` is relative and does not link.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::{bc_raw, build_from_words};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// `addi rT, rA, imm` (opcode 14).
fn addi_raw(rt: u32, ra: u32, imm: i16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | u32::from(imm as u16)
}

/// A plain `bc` (BO 00100) and a `bdnz` (BO 10000), which the shadow
/// quickens, both fuse.
#[test]
fn addi_fuses_with_bc() {
    for bo in [0b00100u8, 0b10000] {
        let shadow = build_from_words(0, &[addi_raw(5, 6, -3), bc_raw(u32::from(bo), 2, -8)]);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::AddiBc {
                rt: 5,
                ra: 6,
                imm: -3,
                bo,
                bi: 2,
                target_offset: -8,
            })),
            "bo {bo:#07b}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

/// [PPC-Book1 p:51 s:3.3.8] addi with RA = 0 adds to 0, and the shadow
/// quickens it to `li`. The pair keeps RA = 0.
#[test]
fn li_fuses_with_bc_as_an_addi_from_zero() {
    let shadow = build_from_words(0, &[addi_raw(5, 0, -3), bc_raw(0b01100, 2, 8)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::AddiBc {
            rt: 5,
            ra: 0,
            imm: -3,
            bo: 0b01100,
            bi: 2,
            target_offset: 8,
        }))
    );
}

#[test]
fn a_linking_or_absolute_bc_does_not_fuse() {
    // bc with LK = 1, then bc with AA = 1.
    for second in [bc_raw(0b01100, 2, 8) | 1, bc_raw(0b01100, 2, 8) | 2] {
        let shadow = build_from_words(0, &[addi_raw(5, 6, -3), second]);
        assert_eq!(
            shadow.get(0),
            Some(I::Addi {
                rt: 5,
                ra: 6,
                imm: -3
            }),
            "{second:#010x}"
        );
        let shadow = build_from_words(0, &[addi_raw(5, 0, -3), second]);
        assert_eq!(
            shadow.get(0),
            Some(I::Li { rt: 5, imm: -3 }),
            "li: {second:#010x}"
        );
    }
}

/// Runs a fused `addi r5, r6, -3; bdnz` at 0x1000 with CTR = `ctr`.
/// Returns the verdict, the new PC, r5 and CTR.
fn run_bdnz(ctr: u64) -> (ExecuteVerdict, u64, u64, u64) {
    let pair = I::Fused(FusedPair::AddiBc {
        rt: 5,
        ra: 6,
        imm: -3,
        bo: 0b10000,
        bi: 0,
        target_offset: -8,
    });
    let mut state = PpuState::new();
    state.pc = 0x1000;
    state.set_gpr(6, 10);
    state.set_ctr(ctr);
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    (verdict, state.pc, state.gpr[5], state.ctr())
}

/// [PPC-Book1 p:51 s:3.3.8] addi writes (RA) + EXTS(SI). [PPC-Book1 p:24
/// s:2.4] bdnz decrements CTR and branches while it is not 0, to the bc
/// slot (0x1004) minus 8.
#[test]
fn the_addi_writes_its_sum_and_the_bdnz_counts_down() {
    assert_eq!(run_bdnz(2), (ExecuteVerdict::Branch, 0xFFC, 7, 1));
    assert_eq!(run_bdnz(1), (ExecuteVerdict::Continue, 0x1000, 7, 0));
}

/// [PPC-Book1 p:51 s:3.3.8] with RA = 0 the addi adds its immediate to 0.
/// The fused `li; bc` writes the immediate and times as the `li` does.
#[test]
fn a_fused_li_bc_ignores_r0() {
    let pair = I::Fused(FusedPair::AddiBc {
        rt: 5,
        ra: 0,
        imm: -3,
        bo: 0b10100,
        bi: 0,
        target_offset: 8,
    });
    let mut state = PpuState::new();
    state.pc = 0x1000;
    state.set_gpr(0, 0x100);
    let ([first, _], _) = crate::timing::classify(&pair, &state);
    let ([li, _], _) = crate::timing::classify(&I::Li { rt: 5, imm: -3 }, &state);
    assert_eq!(first, li);
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    assert_eq!(
        (verdict, state.pc, state.gpr[5]),
        (ExecuteVerdict::Branch, 0x100C, (-3i64) as u64)
    );
}

#[test]
fn a_fused_addi_bc_disassembles_as_its_halves() {
    let text = |ra| {
        let pair = I::Fused(FusedPair::AddiBc {
            rt: 5,
            ra,
            imm: -3,
            bo: 16,
            bi: 0,
            target_offset: -8,
        });
        AsmText {
            insn: &pair,
            addr: 0x1000,
            symbols: None,
        }
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    };
    assert_eq!(text(6), "addi r5, r6, -3; bc 16, 0, 0xffc");
    assert_eq!(text(0), "li r5, -3; bc 16, 0, 0xffc");
}
