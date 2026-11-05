//! `or` (or its quickened `mr`) fuses with a following `add`; a recording
//! `or.` or `add.`, or an `addo`, does not.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{AsmText, PpuInstruction as I};
use crate::shadow::test_support::{build_from_words, or_raw};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// `add rT, rA, rB` (opcode 31, XO 266), with OE and Rc.
fn add_raw(rt: u32, ra: u32, rb: u32, oe: bool, rc: bool) -> u32 {
    (31 << 26)
        | (rt << 21)
        | (ra << 16)
        | (rb << 11)
        | (u32::from(oe) << 10)
        | (266 << 1)
        | u32::from(rc)
}

#[test]
fn or_and_its_mr_form_fuse_with_add() {
    let or = build_from_words(0, &[or_raw(4, 3, 5), add_raw(6, 3, 7, false, false)]);
    assert_eq!(
        or.get(0),
        Some(I::OrAdd {
            ra: 3,
            rs: 4,
            rb: 5,
            rt: 6,
            add_ra: 3,
            add_rb: 7,
        })
    );
    assert_eq!(or.get(4), Some(I::Consumed));
    let mr = build_from_words(0, &[or_raw(4, 3, 4), add_raw(6, 3, 7, false, false)]);
    assert_eq!(
        mr.get(0),
        Some(I::OrAdd {
            ra: 3,
            rs: 4,
            rb: 4,
            rt: 6,
            add_ra: 3,
            add_rb: 7,
        })
    );
}

/// [PPC-Book1 p:67 s:3.3.11] `or.` sets CR0; [PPC-Book1 p:52 s:3.3.8]
/// `add.` sets CR0 and `addo` sets XER OV and SO. The fused arm writes
/// neither, so none of them fuses.
#[test]
fn a_recording_or_or_add_or_an_addo_does_not_fuse() {
    for (first, second) in [
        (or_raw(4, 3, 5) | 1, add_raw(6, 3, 7, false, false)),
        (or_raw(4, 3, 5), add_raw(6, 3, 7, false, true)),
        (or_raw(4, 3, 5), add_raw(6, 3, 7, true, false)),
    ] {
        let shadow = build_from_words(0, &[first, second]);
        assert_eq!(
            shadow.get(4).map(|i| i == I::Consumed),
            Some(false),
            "{first:#010x} {second:#010x}"
        );
    }
}

/// [PPC-Book1 p:67 s:3.3.11] `or rN, rN, rN` is a priority hint, so it
/// stays a raw `or` and does not fuse.
#[test]
fn a_priority_hint_or_does_not_fuse_with_add() {
    let shadow = build_from_words(0, &[or_raw(1, 1, 1), add_raw(6, 3, 7, false, false)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Or {
            ra: 1,
            rs: 1,
            rb: 1,
            rc: false,
        })
    );
}

/// r6 after the pair runs from r4 = 0xF0, r5 = 0x0F, r7 = 0x100.
fn r6_after(insn: &I) -> (u64, ExecuteVerdict) {
    let mut state = PpuState::new();
    state.set_gpr(4, 0xF0);
    state.set_gpr(5, 0x0F);
    state.set_gpr(7, 0x100);
    let verdict = execute(
        insn,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    (state.gpr[6], verdict)
}

/// [PPC-Book1 p:17 s:2.2] the add reads the RA the or just wrote.
#[test]
fn the_add_reads_the_or_result() {
    let pair = I::OrAdd {
        ra: 3,
        rs: 4,
        rb: 5,
        rt: 6,
        add_ra: 3,
        add_rb: 7,
    };
    assert_eq!(r6_after(&pair), (0x1FF, ExecuteVerdict::Continue));
}

#[test]
fn a_fused_or_add_disassembles_as_its_halves() {
    let pair = I::OrAdd {
        ra: 3,
        rs: 4,
        rb: 5,
        rt: 6,
        add_ra: 3,
        add_rb: 7,
    };
    let text = AsmText {
        insn: &pair,
        addr: 0,
        symbols: None,
    }
    .to_string();
    assert_eq!(
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        "or r3, r4, r5; add r6, r3, r7"
    );
}

/// [PPC-Book1 p:163 s:B.9] The `mr` half disassembles as `mr`, the way
/// the standalone `or rA, rS, rS` and the quickened `mr` both render.
#[test]
fn a_fused_mr_add_disassembles_its_first_half_as_mr() {
    let pair = I::OrAdd {
        ra: 3,
        rs: 4,
        rb: 4,
        rt: 6,
        add_ra: 3,
        add_rb: 7,
    };
    let text = AsmText {
        insn: &pair,
        addr: 0,
        symbols: None,
    }
    .to_string();
    assert_eq!(
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        "mr r3, r4; add r6, r3, r7"
    );
}
