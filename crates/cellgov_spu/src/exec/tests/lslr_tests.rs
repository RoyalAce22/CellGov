//! Every local-store address and link goes through the limit register.
//!
//! Each case runs under a 32 KB limit, so an address the full 256 KB
//! limit would leave alone wraps here.

use super::*;
use crate::state::{SpuObservableSnapshot, SpuState, SPU_LSLR_FULL};

// [SPU-ISA p:31 s:3 Table 3-1] 0x00007FFF selects a 32 KB local store.
const LSLR_32K: u32 = 0x7FFF;

fn uid() -> UnitId {
    UnitId::new(0)
}

fn state_32k() -> SpuState {
    let mut s = SpuState::new();
    s.lslr = LSLR_32K;
    s
}

#[test]
fn a_new_state_holds_the_full_limit_and_the_snapshot_carries_it() {
    let s = SpuState::new();
    assert_eq!(s.lslr, SPU_LSLR_FULL);
    assert_eq!(SPU_LSLR_FULL, 0x0003_FFFF);
    assert_eq!(SpuObservableSnapshot::capture(&state_32k()).lslr, LSLR_32K);
}

#[test]
fn a_load_past_the_limit_reads_the_wrapped_quadword() {
    let mut s = state_32k();
    s.ls[0x10..0x20].copy_from_slice(&[0xA5; 16]);
    s.set_reg_word_splat(1, 0x8010);
    let out = execute(
        &SpuInstruction::Lqd {
            rt: 2,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
    );
    assert_eq!(out, SpuStepOutcome::Continue);
    assert_eq!(s.regs[2], [0xA5; 16]);
}

#[test]
fn a_store_past_the_limit_writes_the_wrapped_quadword() {
    let mut s = state_32k();
    s.regs[2] = [0x5A; 16];
    s.set_reg_word_splat(1, 0x8020);
    let out = execute(
        &SpuInstruction::Stqd {
            rt: 2,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
    );
    assert_eq!(out, SpuStepOutcome::Continue);
    assert_eq!(&s.ls[0x20..0x30], &[0x5A; 16]);
    assert!(s.ls[0x8020..0x8030].iter().all(|&b| b == 0));
}

#[test]
fn a_relative_branch_past_the_limit_lands_on_the_wrapped_target() {
    let mut s = state_32k();
    s.pc = 0x7FFC;
    execute(&SpuInstruction::Br { offset: 2 }, &mut s, uid());
    assert_eq!(s.pc, 0x4);
}

#[test]
fn a_link_past_the_limit_is_the_wrapped_address() {
    let mut s = state_32k();
    s.pc = 0x7FFC;
    execute(&SpuInstruction::Brsl { rt: 3, offset: 0 }, &mut s, uid());
    assert_eq!(s.reg_word(3), 0);
    assert_eq!(s.pc, 0x7FFC);
}

#[test]
fn an_indirect_branch_past_the_limit_lands_on_the_wrapped_target() {
    let mut s = state_32k();
    s.set_reg_word_splat(1, 0x800B);
    execute(
        &SpuInstruction::Bi {
            ra: 1,
            d: false,
            e: false,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.pc, 0x8);
}

#[test]
fn every_taken_conditional_relative_branch_lands_on_the_wrapped_target() {
    let cases = [
        SpuInstruction::Brz { rt: 1, offset: 2 },
        SpuInstruction::Brnz { rt: 2, offset: 2 },
        SpuInstruction::Brhz { rt: 1, offset: 2 },
        SpuInstruction::Brhnz { rt: 2, offset: 2 },
    ];
    for insn in cases {
        let mut s = state_32k();
        s.set_reg_word_splat(2, 1);
        s.pc = 0x7FFC;
        assert_eq!(
            execute(&insn, &mut s, uid()),
            SpuStepOutcome::Branch,
            "{insn:?}"
        );
        assert_eq!(s.pc, 0x4, "{insn:?}");
    }
}

#[test]
fn every_taken_conditional_indirect_branch_lands_on_the_wrapped_target() {
    let cases = [
        SpuInstruction::Biz {
            rt: 1,
            ra: 3,
            d: false,
            e: false,
        },
        SpuInstruction::Binz {
            rt: 2,
            ra: 3,
            d: false,
            e: false,
        },
        SpuInstruction::Bihz {
            rt: 1,
            ra: 3,
            d: false,
            e: false,
        },
        SpuInstruction::Bihnz {
            rt: 2,
            ra: 3,
            d: false,
            e: false,
        },
    ];
    for insn in cases {
        let mut s = state_32k();
        s.set_reg_word_splat(2, 1);
        s.set_reg_word_splat(3, 0x800B);
        assert_eq!(
            execute(&insn, &mut s, uid()),
            SpuStepOutcome::Branch,
            "{insn:?}"
        );
        assert_eq!(s.pc, 0x8, "{insn:?}");
    }
}

#[test]
fn bisl_wraps_both_its_target_and_its_link() {
    let mut s = state_32k();
    s.pc = 0x7FFC;
    s.set_reg_word_splat(1, 0x800B);
    execute(
        &SpuInstruction::Bisl {
            rt: 3,
            ra: 1,
            d: false,
            e: false,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.pc, 0x8);
    assert_eq!(s.reg_word(3), 0);
}

#[test]
fn fall_through_and_fetch_wrap_at_the_limit() {
    let mut s = state_32k();
    s.pc = 0x7FFC;
    s.advance_pc();
    assert_eq!(s.pc, 0);

    s.ls[4..8].copy_from_slice(&0x1234_5678u32.to_be_bytes());
    s.pc = 0x8004;
    assert_eq!(s.fetch(), Some(0x1234_5678));
}
