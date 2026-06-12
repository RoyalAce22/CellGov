//! Branch execution: target computation, LR updates, and absolute-address sign extension.

use super::*;
use crate::exec::test_support::exec_no_mem;

#[test]
fn branch_unconditional() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    let result = exec_no_mem(
        &PpuInstruction::B {
            offset: -8,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x0FF8);
}

#[test]
fn bl_sets_lr() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    exec_no_mem(
        &PpuInstruction::B {
            offset: 0x100,
            aa: false,
            link: true,
        },
        &mut s,
    );
    assert_eq!(s.lr, 0x1004);
    assert_eq!(s.pc, 0x1100);
}

#[test]
fn ba_with_negative_offset_sign_extends_to_high_address() {
    // [PPC-Book1 p:24 s:2.4] `NIA <- EXTS(LI || 0b00)` for `ba`.
    // A cast path that zero-extended (e.g. `offset as u32 as
    // u64`) would land at 0x0000_0000_FFFF_FF00 instead of the
    // architectural 0xFFFF_FFFF_FFFF_FF00 -- which is how the
    // hypervisor reaches its high-address exception vectors.
    let mut s = PpuState::new();
    s.pc = 0x2000;
    let result = exec_no_mem(
        &PpuInstruction::B {
            offset: -0x100,
            aa: true,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0xFFFF_FFFF_FFFF_FF00);
}

#[test]
fn ba_branches_to_absolute_address() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    let result = exec_no_mem(
        &PpuInstruction::B {
            offset: 0x100,
            aa: true,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(
        s.pc, 0x100,
        "aa=true: target is offset itself, not PC+offset"
    );
}

#[test]
fn bla_sets_lr_and_branches_absolute() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    exec_no_mem(
        &PpuInstruction::B {
            offset: 0x400,
            aa: true,
            link: true,
        },
        &mut s,
    );
    assert_eq!(s.lr, 0x2004);
    assert_eq!(s.pc, 0x400);
}

#[test]
fn blr_returns_to_lr() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.lr = 0x1000;
    // BO=0x14: always taken, CTR not decremented.
    let result = exec_no_mem(
        &PpuInstruction::Bclr {
            bo: 0x14,
            bi: 0,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x1000);
}

#[test]
fn bc_beq_taken() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    // BO=0x0C: branch on CR true, no CTR decrement. BI=2: EQ bit of cr0.
    s.set_cr_field(0, 0b0010);
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 8,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x1008);
}

#[test]
fn bc_beq_not_taken() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.set_cr_field(0, 0b0100); // GT
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 8,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Continue));
    assert_eq!(s.pc, 0x1000);
}

#[test]
fn bca_branches_to_absolute_address() {
    // bca with aa=true: target is sign-extended BD || 0b00, NOT
    // PC + BD. A regression that ignored aa would compute
    // 0x2000 + 0x100 = 0x2100 instead of 0x100.
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.set_cr_field(0, 0b0010); // EQ
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 0x100,
            aa: true,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x100);
}

#[test]
fn bcla_sets_lr_and_branches_absolute() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.set_cr_field(0, 0b0010); // EQ
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 0x400,
            aa: true,
            link: true,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.lr, 0x2004);
    assert_eq!(s.pc, 0x400);
}

#[test]
fn bcl_sets_lr_even_when_branch_not_taken() {
    // [PPC-Book1 p:24 s:2.4] LK=1 writes LR regardless of whether
    // the branch is taken. A regression that gated the LR write
    // on cond_ok would leave LR unchanged here.
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.lr = 0xDEAD;
    s.set_cr_field(0, 0b0100); // GT, so EQ test fails
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 8,
            aa: false,
            link: true,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Continue));
    assert_eq!(s.pc, 0x1000, "PC unchanged when branch not taken");
    assert_eq!(s.lr, 0x1004, "LR still written under LK=1");
}

#[test]
fn bdnz_decrements_ctr_and_branches_when_nonzero() {
    // BO=0x10: skip CR test, decrement CTR, branch if CTR != 0.
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.ctr = 3;
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x10,
            bi: 0,
            offset: -16,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.ctr, 2);
    assert_eq!(s.pc, 0x0FF0);
}

#[test]
fn bdnz_falls_through_when_ctr_decrements_to_zero() {
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.ctr = 1;
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x10,
            bi: 0,
            offset: -16,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Continue));
    assert_eq!(s.ctr, 0);
    assert_eq!(s.pc, 0x1000);
}

#[test]
fn blrl_branches_to_old_lr_and_writes_new_lr() {
    // Old LR is the branch target; new LR points at the
    // following instruction. A regression that wrote LR before
    // capturing target would jump to PC+4 instead of the caller.
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.lr = 0x1000;
    let result = exec_no_mem(
        &PpuInstruction::Bclr {
            bo: 0x14,
            bi: 0,
            link: true,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x1000, "branched to original LR");
    assert_eq!(s.lr, 0x2004, "new LR = caller PC + 4");
}

#[test]
fn bcctr_taken_branches_to_ctr() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.ctr = 0x4000;
    let result = exec_no_mem(
        &PpuInstruction::Bcctr {
            bo: 0x14,
            bi: 0,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x4000);
    assert_eq!(s.ctr, 0x4000, "bcctr does not decrement CTR");
}

#[test]
fn bcctr_not_taken_when_cr_false() {
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.ctr = 0x4000;
    s.set_cr_field(0, 0b0100); // GT, so EQ test fails
    let result = exec_no_mem(
        &PpuInstruction::Bcctr {
            bo: 0x0C,
            bi: 2,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Continue));
    assert_eq!(s.pc, 0x2000);
    assert_eq!(s.ctr, 0x4000);
}

#[test]
fn bcctr_does_not_decrement_ctr_even_with_bo_2_clear() {
    // BO=0x00 has the "decrement CTR" bit clear, which would be
    // the bdnzctr invalid form. We treat BO_2 as don't-care for
    // bcctr so CTR is not corrupted before being used as target.
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.ctr = 0x4000;
    s.set_cr_field(0, 0b0010); // EQ, so EQ test passes
    let result = exec_no_mem(
        &PpuInstruction::Bcctr {
            bo: 0x08, // CR polarity=true, EQ test
            bi: 2,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(
        s.ctr, 0x4000,
        "CTR untouched even when BO_2=0 (invalid form)"
    );
    assert_eq!(s.pc, 0x4000);
}

#[test]
fn bc_lk_zero_leaves_lr_untouched() {
    // LK=0 must not write LR even when the branch is taken.
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.lr = 0xDEAD;
    s.set_cr_field(0, 0b0010); // EQ -> branch taken
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x0C,
            bi: 2,
            offset: 8,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.lr, 0xDEAD, "LR untouched when LK=0");
}

#[test]
fn bc_ctr_decrement_happens_when_bo2_clear() {
    // BO=0x08 has BO_2 (0x04) clear -> CTR decrements regardless of
    // taken/not-taken. BO_0=0 enables CR test, BO_1=1 requires
    // CR_BI=1 to branch; with the CR bit clear the branch is not
    // taken, but CTR must still decrement.
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.ctr = 7;
    s.set_cr_field(0, 0b0100); // GT only; EQ bit (bi=2) is 0
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x08, // BO_2=0 decrement; require CR_BI=1, which it isn't
            bi: 2,
            offset: 8,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Continue));
    assert_eq!(s.ctr, 6, "CTR decremented before condition check");
}

#[test]
fn bc_ctr_no_decrement_when_bo2_set() {
    // BO=0x14: BO_2 (0x04) set -> CTR untouched.
    let mut s = PpuState::new();
    s.pc = 0x1000;
    s.ctr = 5;
    let result = exec_no_mem(
        &PpuInstruction::Bc {
            bo: 0x14,
            bi: 0,
            offset: 8,
            aa: false,
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.ctr, 5, "CTR unchanged when BO_2=1");
}

#[test]
fn bclr_reads_cr_bit_in_cr1_field() {
    // BI >= 4 selects fields beyond CR0; cr_bit indexing must
    // honor the full BI range.
    let mut s = PpuState::new();
    s.pc = 0x2000;
    s.lr = 0x1000;
    // CR1 EQ bit lives at PPC bit 6 (4*1 + 2). set_cr_field
    // takes the field index, not the bit.
    s.set_cr_field(1, 0b0010);
    let result = exec_no_mem(
        &PpuInstruction::Bclr {
            bo: 0x0C,
            bi: 6, // CR1 EQ
            link: false,
        },
        &mut s,
    );
    assert!(matches!(result, ExecuteVerdict::Branch));
    assert_eq!(s.pc, 0x1000);
}
