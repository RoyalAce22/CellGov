//! `lis rT, hi` fuses with a following `ori rT, rT, lo` into one constant.
//! These second halves do not fuse:
//! - an `ori` on a different register
//! - the preferred no-op
//! - an `addi`

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::{build_from_words, ori_raw};
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// `addis rT, 0, hi` (opcode 15).
fn lis_raw(rt: u32, hi: i16) -> u32 {
    (15 << 26) | (rt << 21) | u32::from(hi as u16)
}

/// The halfwords at and either side of each sign boundary.
const HALVES: [u16; 4] = [0x0000, 0x7FFF, 0x8000, 0xFFFF];

#[test]
fn lis_fuses_with_a_same_register_ori() {
    let shadow = build_from_words(0, &[lis_raw(3, 0x1234), ori_raw(3, 3, 0xBEEF)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LisOri {
            rt: 3,
            hi: 0x1234,
            lo: 0xBEEF,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

#[test]
fn an_ori_on_another_register_does_not_fuse() {
    for second in [ori_raw(3, 4, 1), ori_raw(4, 3, 1)] {
        let shadow = build_from_words(0, &[lis_raw(3, 1), second]);
        assert!(
            matches!(shadow.get(0), Some(I::Lis { .. })),
            "{second:#010x}"
        );
    }
}

/// [PPC-Book1 p:162 s:B.9] `ori 0, 0, 0` is the preferred no-op.
#[test]
fn the_preferred_no_op_does_not_fuse() {
    let shadow = build_from_words(0, &[lis_raw(0, 1), ori_raw(0, 0, 0)]);
    assert_eq!(shadow.get(4), Some(I::Nop));
    let nop = I::Ori {
        ra: 0,
        rs: 0,
        imm: 0,
    };
    assert_eq!(super::make_super_pair(I::Lis { rt: 0, imm: 1 }, nop), None);
}

#[test]
fn a_same_register_addi_does_not_fuse() {
    let addi = I::Addi {
        rt: 3,
        ra: 3,
        imm: 1,
    };
    assert_eq!(super::make_super_pair(I::Lis { rt: 3, imm: 1 }, addi), None);
}

/// r3 and the verdicts after `insns` run in order from r3 = all ones.
fn run(insns: &[I]) -> (u64, Vec<ExecuteVerdict>) {
    let mut state = PpuState::new();
    state.set_gpr(3, u64::MAX);
    let verdicts = insns
        .iter()
        .map(|insn| {
            execute(
                insn,
                &mut state,
                cellgov_event::UnitId::new(0),
                &[],
                &mut Vec::new(),
                &mut StoreBuffer::new(),
            )
        })
        .collect();
    (state.gpr[3], verdicts)
}

#[test]
fn a_fused_constant_matches_its_halves_at_the_sign_boundaries() {
    for hi in HALVES.map(|h| h as i16) {
        for lo in HALVES {
            let lis = I::Lis { rt: 3, imm: hi };
            let ori = I::Ori {
                ra: 3,
                rs: 3,
                imm: lo,
            };
            let pair = super::make_super_pair(lis, ori).expect("lis; ori fuses");
            let (halves, _) = run(&[lis, ori]);
            assert_eq!(
                run(&[pair]),
                (halves, vec![ExecuteVerdict::Continue]),
                "{pair:?}"
            );
        }
    }
}

#[test]
fn a_fused_constant_disassembles_as_its_halves() {
    let text = |insn: I| {
        AsmText {
            insn: &insn,
            addr: 0,
            symbols: None,
        }
        .to_string()
    };
    for hi in HALVES.map(|h| h as i16) {
        for lo in HALVES {
            let lis = I::Lis { rt: 3, imm: hi };
            let ori = I::Ori {
                ra: 3,
                rs: 3,
                imm: lo,
            };
            assert_eq!(
                super::make_super_pair(lis, ori).map(text),
                Some(format!("{}; ori r3, r3, 0x{lo:x}", text(lis)))
            );
        }
    }
}
