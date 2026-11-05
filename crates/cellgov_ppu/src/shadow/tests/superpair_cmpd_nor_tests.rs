//! `cmpd` fuses with a following `nor`; a recording `nor.` does not.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// `cmpd crF, rA, rB` (opcode 31, XO 0, L = 1).
fn cmpd_raw(bf: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (bf << 23) | (1 << 21) | (ra << 16) | (rb << 11)
}

/// `nor rA, rS, rB` (opcode 31, XO 124), with Rc from `rc`.
fn nor_raw(ra: u32, rs: u32, rb: u32, rc: bool) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (124 << 1) | u32::from(rc)
}

#[test]
fn cmpd_fuses_with_nor() {
    let shadow = build_from_words(0, &[cmpd_raw(7, 3, 4), nor_raw(5, 6, 6, false)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::CmpdNor {
            bf: 7,
            ra: 3,
            rb: 4,
            nor_ra: 5,
            nor_rs: 6,
            nor_rb: 6,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

/// [PPC-Book1 p:68 s:3.3.11] `nor.` sets CR0, which the fused arm does
/// not write, so it does not fuse.
#[test]
fn a_recording_nor_does_not_fuse() {
    let shadow = build_from_words(0, &[cmpd_raw(7, 3, 4), nor_raw(5, 6, 6, true)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Cmpd {
            bf: 7,
            ra: 3,
            rb: 4
        })
    );
    assert_eq!(
        shadow.get(4),
        Some(I::Nor {
            ra: 5,
            rs: 6,
            rb: 6,
            rc: true
        })
    );
}

/// [PPC-Book1 p:60 s:3.3.9] cmpd compares all 64 bits: 0x1_0000_0000 is
/// greater than 1, though its low word is less. The nor then writes
/// ~(r6 | r6).
#[test]
fn the_compare_reads_64_bits_and_the_nor_writes_its_result() {
    let pair = I::Fused(FusedPair::CmpdNor {
        bf: 7,
        ra: 3,
        rb: 4,
        nor_ra: 5,
        nor_rs: 6,
        nor_rb: 6,
    });
    let mut state = PpuState::new();
    state.set_gpr(3, 0x1_0000_0000);
    state.set_gpr(4, 1);
    state.set_gpr(6, 0xF0);
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.cr_field(7), 0b0100, "cr7 is GT");
    assert_eq!(state.gpr[5], !0xF0);
}

#[test]
fn a_fused_cmpd_nor_disassembles_as_its_halves() {
    let pair = |bf| {
        I::Fused(FusedPair::CmpdNor {
            bf,
            ra: 3,
            rb: 4,
            nor_ra: 5,
            nor_rs: 6,
            nor_rb: if bf == 1 { 6 } else { 7 },
        })
    };
    let text = |insn: &I| {
        AsmText {
            insn,
            addr: 0,
            symbols: None,
        }
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    };
    assert_eq!(text(&pair(0)), "cmpd r3, r4; nor r5, r6, r7");
    assert_eq!(text(&pair(7)), "cmpd cr7, r3, r4; nor r5, r6, r7");
    assert_eq!(text(&pair(1)), "cmpd cr1, r3, r4; not r5, r6");
}
