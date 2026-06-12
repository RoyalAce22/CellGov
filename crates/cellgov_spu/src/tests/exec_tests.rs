//! Per-instruction SPU execute semantics -- immediate loads, arithmetic, and shuffles on vector registers.

use super::*;
use crate::state::SpuState;

fn uid() -> UnitId {
    UnitId::new(0)
}

#[test]
fn il_splats_all_slots() {
    let mut s = SpuState::new();
    execute(&SpuInstruction::Il { rt: 5, imm: -1 }, &mut s, uid());
    for slot in 0..4 {
        assert_eq!(s.reg_word_slot(5, slot), 0xFFFFFFFF);
    }
}

#[test]
fn ilhu_shifts_left_16() {
    let mut s = SpuState::new();
    execute(&SpuInstruction::Ilhu { rt: 3, imm: 0x1337 }, &mut s, uid());
    assert_eq!(s.reg_word(3), 0x13370000);
}

#[test]
fn iohl_ors_lower_halfword() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(3, 0x13370000);
    execute(&SpuInstruction::Iohl { rt: 3, imm: 0xBAAD }, &mut s, uid());
    assert_eq!(s.reg_word(3), 0x1337BAAD);
}

#[test]
fn sf_subtracts_ra_from_rb() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 3);
    s.set_reg_word_splat(2, 10);
    execute(
        &SpuInstruction::Sf {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(3), 7);
}

#[test]
fn ori_with_zero_is_move() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0xDEADBEEF);
    execute(
        &SpuInstruction::Ori {
            rt: 2,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(2), 0xDEADBEEF);
}

#[test]
fn shufb_identity_pattern() {
    let mut s = SpuState::new();
    s.regs[1] = [
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F,
    ];
    s.regs[3] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    execute(
        &SpuInstruction::Shufb {
            rt: 4,
            ra: 1,
            rb: 2,
            rc: 3,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.regs[4], s.regs[1]);
}

#[test]
fn bi_branches_to_register() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(0, 0x3A0);
    let outcome = execute(&SpuInstruction::Bi { ra: 0 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Branch));
    assert_eq!(s.pc, 0x3A0);
}

#[test]
fn brsl_sets_link_and_branches() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    let outcome = execute(&SpuInstruction::Brsl { rt: 0, offset: 16 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Branch));
    assert_eq!(s.reg_word(0), 0x104);
    assert_eq!(s.pc, 0x140);
}

#[test]
fn cwd_generates_word_insertion_mask() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(3, 0x3000);
    execute(
        &SpuInstruction::Cwd {
            rt: 5,
            ra: 3,
            imm: 0,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.regs[5][0], 0x00);
    assert_eq!(s.regs[5][1], 0x01);
    assert_eq!(s.regs[5][2], 0x02);
    assert_eq!(s.regs[5][3], 0x03);
    assert_eq!(s.regs[5][4], 0x14);
}

#[test]
fn lqa_stqa_roundtrip() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(5, 0xCAFEBABE);
    let addr_imm: i16 = (0x3000u16 >> 2) as i16;
    execute(
        &SpuInstruction::Stqa {
            rt: 5,
            imm: addr_imm,
        },
        &mut s,
        uid(),
    );
    execute(
        &SpuInstruction::Lqa {
            rt: 6,
            imm: addr_imm,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.regs[6], s.regs[5]);
}

#[test]
fn fsmbi_creates_byte_mask() {
    let mut s = SpuState::new();
    execute(&SpuInstruction::Fsmbi { rt: 3, imm: 0xFFFF }, &mut s, uid());
    assert!(s.regs[3].iter().all(|&b| b == 0xFF));
    execute(&SpuInstruction::Fsmbi { rt: 4, imm: 0x0000 }, &mut s, uid());
    assert!(s.regs[4].iter().all(|&b| b == 0x00));
}

#[test]
fn stop_yields_finished() {
    let mut s = SpuState::new();
    let outcome = execute(&SpuInstruction::Stop { signal: 0 }, &mut s, uid());
    assert!(matches!(
        outcome,
        SpuStepOutcome::Yield {
            reason: YieldReason::Finished,
            ..
        }
    ));
}

#[test]
fn nop_continues() {
    let mut s = SpuState::new();
    assert!(matches!(
        execute(&SpuInstruction::Nop, &mut s, uid()),
        SpuStepOutcome::Continue
    ));
    assert!(matches!(
        execute(&SpuInstruction::Lnop, &mut s, uid()),
        SpuStepOutcome::Continue
    ));
}
