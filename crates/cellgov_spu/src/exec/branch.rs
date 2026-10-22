//! Branch and halt: the relative, absolute and indirect branches, their
//! conditional forms, and the conditional halts.

use super::outcome::SpuStepOutcome;
use crate::state::SpuState;
use crate::stop::SpuStopKind;

/// The taken path of every indirect branch: PC <- `target`, with the
/// D and E feature bits applied to the interrupt-enable state.
///
/// [SPU-ISA p:251 s:12] a taken branch changes the interrupt-enable status before the target instruction runs, and D = E = 1 causes undefined behavior.
// The `spu_run_control` and `spu_ie_branch_forms` console captures: every
// taken form with D and E both set disables interrupts, from the enabled
// state and from the disabled one.
fn take_indirect(state: &mut SpuState, target: u32, d: bool, e: bool) -> SpuStepOutcome {
    match (d, e) {
        (true, _) => state.set_interrupts_enabled(false),
        (false, true) => state.set_interrupts_enabled(true),
        (false, false) => {}
    }
    state.pc = target;
    SpuStepOutcome::Branch
}

/// The shared body of the indirect conditional branches.
fn branch_indirect_if(
    state: &mut SpuState,
    ra: u8,
    d: bool,
    e: bool,
    taken: bool,
) -> SpuStepOutcome {
    if taken {
        let target = state.insn_addr(state.reg_word(ra));
        take_indirect(state, target, d, e)
    } else {
        SpuStepOutcome::Continue
    }
}

/// A halt instruction's outcome: a stop of kind `Halt` when its condition
/// holds, else the next instruction.
///
/// The ISA lets the SPU run zero or more instructions past a met halt;
/// CellGov runs none, so the resume address is the word after the halt.
///
/// [SPU-ISA p:149 s:7] a halt stops imprecisely, at or after the halt instruction.
fn halt_if(condition: bool) -> SpuStepOutcome {
    if condition {
        SpuStepOutcome::Stop {
            kind: SpuStopKind::Halt,
            signal: 0,
        }
    } else {
        SpuStepOutcome::Continue
    }
}

/// [SPU-ISA p:174 s:7. Compare, Branch, and Halt Instructions] Branch Relative: PC <- PC + sign-extended I16<<2, masked to LS range.
pub(super) fn br(state: &mut SpuState, offset: i32) -> SpuStepOutcome {
    state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
    SpuStepOutcome::Branch
}

/// [SPU-ISA p:176 s:7. Compare, Branch, and Halt Instructions] Branch Relative and Set Link: the link is (PC+4) masked by LSLR in RT's preferred slot with the other slots zeroed, then the relative branch is taken.
pub(super) fn brsl(state: &mut SpuState, rt: u8, offset: i32) -> SpuStepOutcome {
    let link = state.ls_wrap(state.pc.wrapping_add(4));
    state.set_reg(rt as usize, [0u8; 16]);
    state.set_reg_word_slot(rt, 0, link);
    state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
    SpuStepOutcome::Branch
}

/// [SPU-ISA p:175 s:7. Compare, Branch, and Halt Instructions] Branch Absolute: PC <- sign-extended I16<<2, masked to LS range.
pub(super) fn bra(state: &mut SpuState, address: i32) -> SpuStepOutcome {
    state.pc = state.insn_addr((address << 2) as u32);
    SpuStepOutcome::Branch
}

/// [SPU-ISA p:177 s:7. Compare, Branch, and Halt Instructions] Branch Absolute and Set Link: the link is (PC+4) masked by LSLR in RT's preferred slot with the other slots zeroed, then the absolute branch is taken.
pub(super) fn brasl(state: &mut SpuState, rt: u8, address: i32) -> SpuStepOutcome {
    let link = state.ls_wrap(state.pc.wrapping_add(4));
    state.set_reg(rt as usize, [0u8; 16]);
    state.set_reg_word_slot(rt, 0, link);
    state.pc = state.insn_addr((address << 2) as u32);
    SpuStepOutcome::Branch
}

/// [SPU-ISA p:183 s:7. Compare, Branch, and Halt Instructions] Branch If Zero Word: branch when RT preferred slot is zero.
pub(super) fn brz(state: &mut SpuState, rt: u8, offset: i32) -> SpuStepOutcome {
    if state.reg_word(rt) == 0 {
        state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
        SpuStepOutcome::Branch
    } else {
        SpuStepOutcome::Continue
    }
}

/// [SPU-ISA p:182 s:7. Compare, Branch, and Halt Instructions] Branch If Not Zero Word: branch when RT preferred slot is non-zero.
pub(super) fn brnz(state: &mut SpuState, rt: u8, offset: i32) -> SpuStepOutcome {
    if state.reg_word(rt) != 0 {
        state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
        SpuStepOutcome::Branch
    } else {
        SpuStepOutcome::Continue
    }
}

/// [SPU-ISA p:178 s:7. Compare, Branch, and Halt Instructions] Branch Indirect: PC <- RA preferred slot masked to LS range.
pub(super) fn bi(state: &mut SpuState, ra: u8, d: bool, e: bool) -> SpuStepOutcome {
    let target = state.insn_addr(state.reg_word(ra));
    take_indirect(state, target, d, e)
}

/// [SPU-ISA p:179 s:7. Compare, Branch, and Halt Instructions] Interrupt Return: PC <- SRR0, with the D and E feature bits; RA is ignored.
pub(super) fn iret(state: &mut SpuState, d: bool, e: bool) -> SpuStepOutcome {
    let target = state.insn_addr(state.srr0());
    take_indirect(state, target, d, e)
}

/// [SPU-ISA p:181 s:7. Compare, Branch, and Halt Instructions] Branch Indirect and Set Link: the target is read from RA before RT is written; the link is (PC+4) masked by LSLR in RT's preferred slot with the other slots zeroed, then PC <- RA masked to LS range.
pub(super) fn bisl(state: &mut SpuState, rt: u8, ra: u8, d: bool, e: bool) -> SpuStepOutcome {
    let target = state.insn_addr(state.reg_word(ra));
    let link = state.ls_wrap(state.pc.wrapping_add(4));
    state.set_reg(rt as usize, [0u8; 16]);
    state.set_reg_word_slot(rt, 0, link);
    take_indirect(state, target, d, e)
}

/// [SPU-ISA p:180 s:7. Compare, Branch, and Halt Instructions] Branch Indirect and Set Link if External Data: RT gets the link whether or not the branch is taken, and PC <- RA only when the external condition is true.
/// [CBEA p:147 s:9.11.1] the condition is a non-zero SPU_RdEventStat count.
// The `spu_bisled_d_and_e` console capture: with D and E both set, a
// branch not taken leaves the interrupt-enable state as it was.
pub(super) fn bisled(state: &mut SpuState, rt: u8, ra: u8, d: bool, e: bool) -> SpuStepOutcome {
    let target = state.insn_addr(state.reg_word(ra));
    let link = state.ls_wrap(state.pc.wrapping_add(4));
    state.set_reg(rt as usize, [0u8; 16]);
    state.set_reg_word_slot(rt, 0, link);
    if state.channels.event_count {
        take_indirect(state, target, d, e)
    } else {
        SpuStepOutcome::Continue
    }
}

/// [SPU-ISA p:184 s:7. Compare, Branch, and Halt Instructions] Branch If Not Zero Halfword: branch when the low halfword of RT's preferred slot is non-zero.
pub(super) fn brhnz(state: &mut SpuState, rt: u8, offset: i32) -> SpuStepOutcome {
    if state.reg_word(rt) & 0xFFFF != 0 {
        state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
        SpuStepOutcome::Branch
    } else {
        SpuStepOutcome::Continue
    }
}

/// [SPU-ISA p:185 s:7. Compare, Branch, and Halt Instructions] Branch If Zero Halfword: branch when the low halfword of RT's preferred slot is zero.
pub(super) fn brhz(state: &mut SpuState, rt: u8, offset: i32) -> SpuStepOutcome {
    if state.reg_word(rt) & 0xFFFF == 0 {
        state.pc = state.insn_addr(state.pc.wrapping_add((offset << 2) as u32));
        SpuStepOutcome::Branch
    } else {
        SpuStepOutcome::Continue
    }
}

/// [SPU-ISA p:186 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Zero: PC <- RA preferred slot masked to LS range when RT's preferred word is zero.
pub(super) fn biz(state: &mut SpuState, rt: u8, ra: u8, d: bool, e: bool) -> SpuStepOutcome {
    branch_indirect_if(state, ra, d, e, state.reg_word(rt) == 0)
}

/// [SPU-ISA p:187 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Not Zero: taken when RT's preferred word is non-zero.
pub(super) fn binz(state: &mut SpuState, rt: u8, ra: u8, d: bool, e: bool) -> SpuStepOutcome {
    branch_indirect_if(state, ra, d, e, state.reg_word(rt) != 0)
}

/// [SPU-ISA p:188 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Zero Halfword: taken when the low halfword of RT's preferred slot is zero.
pub(super) fn bihz(state: &mut SpuState, rt: u8, ra: u8, d: bool, e: bool) -> SpuStepOutcome {
    branch_indirect_if(state, ra, d, e, state.reg_word(rt) & 0xFFFF == 0)
}

/// [SPU-ISA p:189 s:7. Compare, Branch, and Halt Instructions] Branch Indirect If Not Zero Halfword: taken when the low halfword of RT's preferred slot is non-zero.
pub(super) fn bihnz(state: &mut SpuState, rt: u8, ra: u8, d: bool, e: bool) -> SpuStepOutcome {
    branch_indirect_if(state, ra, d, e, state.reg_word(rt) & 0xFFFF != 0)
}

/// [SPU-ISA p:150 s:7. Compare, Branch, and Halt Instructions] Halt If Equal: stop when RA's preferred word equals RB's.
pub(super) fn heq(state: &mut SpuState, ra: u8, rb: u8) -> SpuStepOutcome {
    halt_if(state.reg_word(ra) == state.reg_word(rb))
}

/// [SPU-ISA p:151 s:7. Compare, Branch, and Halt Instructions] Halt If Equal Immediate: I10 sign-extended to 32 bits.
pub(super) fn heqi(state: &mut SpuState, ra: u8, imm: i16) -> SpuStepOutcome {
    halt_if(state.reg_word(ra) == imm as i32 as u32)
}

/// [SPU-ISA p:152 s:7. Compare, Branch, and Halt Instructions] Halt If Greater Than: an algebraic compare.
pub(super) fn hgt(state: &mut SpuState, ra: u8, rb: u8) -> SpuStepOutcome {
    halt_if(state.reg_word(ra) as i32 > state.reg_word(rb) as i32)
}

/// [SPU-ISA p:153 s:7. Compare, Branch, and Halt Instructions] Halt If Greater Than Immediate: algebraic, against the sign-extended I10.
pub(super) fn hgti(state: &mut SpuState, ra: u8, imm: i16) -> SpuStepOutcome {
    halt_if(state.reg_word(ra) as i32 > i32::from(imm))
}

/// [SPU-ISA p:154 s:7. Compare, Branch, and Halt Instructions] Halt If Logically Greater Than: an unsigned compare.
pub(super) fn hlgt(state: &mut SpuState, ra: u8, rb: u8) -> SpuStepOutcome {
    halt_if(state.reg_word(ra) > state.reg_word(rb))
}

/// [SPU-ISA p:155 s:7. Compare, Branch, and Halt Instructions] Halt If Logically Greater Than Immediate: unsigned, against the sign-extended I10.
pub(super) fn hlgti(state: &mut SpuState, ra: u8, imm: i16) -> SpuStepOutcome {
    halt_if(state.reg_word(ra) > imm as i32 as u32)
}
