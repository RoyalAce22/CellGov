use super::*;
use crate::decode::decode;

/// Issue `words` at consecutive addresses from `at`, and return each cycle.
///
/// Every branch in `words` falls through.
fn issue_run(at: u32, words: &[u32]) -> Vec<u64> {
    let mut pipeline = SpuPipeline::new();
    let mut state = SpuState::new();
    words
        .iter()
        .enumerate()
        .map(|(i, &raw)| {
            state.pc = at + 4 * i as u32;
            let insn = decode(raw).expect("an SPU instruction");
            let slot = pipeline.plan(&state, raw, &insn);
            pipeline.retire(slot, None);
            slot.cycle
        })
        .collect()
}

const NOP: u32 = 0x4020_007F;
const LNOP: u32 = 0x0020_0000;
/// ai $3,$3,-1
const AI: u32 = 0x1CFF_C183;
/// ai $4,$5,1
const AI_OTHER: u32 = 0x1C00_4284;
/// dfa $5,$6,$7
const DFA: u32 = 0x5981_C305;
/// rchcnt $10,$ch0
const RCHCNT: u32 = 0x01E0_000A;
/// a $3,$3,$10
const A_ON_COUNT: u32 = 0x1802_8183;

#[test]
fn an_even_then_odd_pair_at_a_doubleword_dual_issues() {
    // [CBE-Handbook p:772 s:B.1.3.2 Table B-3] the even/odd pair at x'0000' and x'0020' dual-issue.
    assert_eq!(issue_run(0, &[NOP, LNOP, NOP, LNOP]), vec![0, 0, 1, 1]);
    // An odd instruction first in its fetch group, or an even one second,
    // issues alone.
    assert_eq!(issue_run(4, &[NOP, LNOP, NOP, LNOP]), vec![0, 1, 2, 3]);
    assert_eq!(issue_run(0, &[LNOP, NOP]), vec![0, 1]);
}

#[test]
fn an_instruction_waits_for_the_latency_of_the_result_it_reads() {
    // ai has a latency of 2: the next ai on its result issues 2 later.
    assert_eq!(issue_run(0, &[AI, AI]), vec![0, 2]);
    // An independent one issues at once.
    assert_eq!(issue_run(0, &[AI, AI_OTHER]), vec![0, 1]);
    // rchcnt has a latency of 6.
    assert_eq!(issue_run(4, &[RCHCNT, A_ON_COUNT]), vec![0, 6]);
}

#[test]
fn nothing_issues_for_six_cycles_after_a_double_precision_instruction() {
    assert_eq!(issue_run(0, &[DFA, LNOP, NOP]), vec![0, 7, 8]);
}

#[test]
fn a_second_channel_access_within_five_cycles_is_retried_after_a_refetch() {
    let cycles = issue_run(0, &[RCHCNT, RCHCNT]);
    assert_eq!(cycles, vec![0, 1 + 18]);
    // Six nops put the second six cycles after the first, dual-issued
    // with the last nop.
    let mut spaced = vec![RCHCNT];
    spaced.extend([NOP; 6]);
    spaced.push(RCHCNT);
    assert_eq!(issue_run(0, &spaced).last(), Some(&6));
}

#[test]
fn a_mispredicted_branch_costs_one_more_cycle_into_the_second_half_of_a_fetch_line() {
    let state = SpuState::new();
    // br to itself, not hinted.
    let br = 0x3200_0000;
    let insn = decode(br).expect("br");
    for (target, cost) in [(0x00, 18), (0x3C, 18), (0x40, 19), (0x7C, 19), (0x80, 18)] {
        let mut pipeline = SpuPipeline::new();
        let slot = pipeline.plan(&state, br, &insn);
        pipeline.retire(slot, Some(target));
        assert_eq!(
            pipeline.next_issue(),
            slot.cycle + cost,
            "target {target:#x}"
        );
    }
}

#[test]
fn a_hinted_branch_taken_to_its_target_costs_nothing() {
    let mut state = SpuState::new();
    let mut pipeline = SpuPipeline::new();
    // hbrr at 0 for the branch at 0x40, target 0x100.
    let hbrr = 0x1200_0000 | (0x40 >> 2) | ((0x100 >> 2) << 7);
    let insn = decode(hbrr).expect("hbrr");
    let slot = pipeline.plan(&state, hbrr, &insn);
    pipeline.retire(slot, None);
    // The branch, long after the hint is ready.
    pipeline.charge(100);
    state.pc = 0x40;
    let br = 0x3200_0000 | (((0x100 - 0x40) >> 2) << 7);
    let insn = decode(br).expect("br");
    let slot = pipeline.plan(&state, br, &insn);
    pipeline.retire(slot, Some(0x100));
    assert_eq!(pipeline.next_issue(), slot.cycle + 1);
    // A second pass of the same branch still finds the hint.
    let again = pipeline.plan(&state, br, &insn);
    pipeline.retire(again, None);
    assert_eq!(
        pipeline.next_issue(),
        again.cycle + SPU_HINTED_NOT_TAKEN_CYCLES
    );
}

#[test]
fn a_stallable_hint_holds_its_branch_until_the_hint_is_ready() {
    let mut state = SpuState::new();
    let mut pipeline = SpuPipeline::new();
    // The branch 12 instructions after the hint, back to the hint.
    let hbrr = 0x1200_0000 | 12;
    let insn = decode(hbrr).expect("hbrr");
    let hint = pipeline.plan(&state, hbrr, &insn);
    pipeline.retire(hint, None);
    state.pc = 48;
    let br = 0x3200_0000 | ((-12i32 as u32 & 0xFFFF) << 7);
    let insn = decode(br).expect("br");
    let slot = pipeline.plan(&state, br, &insn);
    assert_eq!(slot.cycle, hint.cycle + SPU_HINT_READY_CYCLES);
    pipeline.retire(slot, Some(0));
    assert_eq!(pipeline.next_issue(), slot.cycle + 1, "predicted");
}

#[test]
fn a_sync_holds_the_next_issue_and_drops_the_hint() {
    let mut state = SpuState::new();
    let mut pipeline = SpuPipeline::new();
    let hbrr = 0x1200_0000 | (0x40 >> 2) | ((0x100 >> 2) << 7);
    let insn = decode(hbrr).expect("hbrr");
    let slot = pipeline.plan(&state, hbrr, &insn);
    pipeline.retire(slot, None);
    pipeline.charge(100);
    state.pc = 0x20;
    let sync = 0x0040_0000;
    let insn = decode(sync).expect("sync");
    let slot = pipeline.plan(&state, sync, &insn);
    pipeline.retire(slot, None);
    assert_eq!(pipeline.next_issue(), slot.cycle + 18, "the refetch");
    state.pc = 0x40;
    let br = 0x3200_0000 | (((0x100 - 0x40) >> 2) << 7);
    let insn = decode(br).expect("br");
    let slot = pipeline.plan(&state, br, &insn);
    pipeline.retire(slot, Some(0x100));
    assert_eq!(pipeline.next_issue(), slot.cycle + 18, "the hint is gone");
}

/// Issue the hint `raw` at `pc`, then let `gap` cycles pass.
fn hint_then_wait(pipeline: &mut SpuPipeline, state: &mut SpuState, pc: u32, raw: u32, gap: u64) {
    state.pc = pc;
    let insn = decode(raw).expect("a hint");
    let slot = pipeline.plan(state, raw, &insn);
    pipeline.retire(slot, None);
    pipeline.charge(gap);
}

#[test]
fn a_ready_hint_replaces_the_loaded_one_before_anything_retires() {
    let mut state = SpuState::new();
    let mut pipeline = SpuPipeline::new();
    // Hint A at 0 for the branch at 0x40, target 0x100, loaded by a nop.
    let hint_a = 0x1200_0000 | (0x40 >> 2) | ((0x100 >> 2) << 7);
    hint_then_wait(&mut pipeline, &mut state, 0, hint_a, 100);
    let insn = decode(NOP).expect("nop");
    state.pc = 4;
    let slot = pipeline.plan(&state, NOP, &insn);
    pipeline.retire(slot, None);
    // Hint B at 8 for the branch at 0x80, ready by the time the branch A
    // named issues.
    let hint_b = 0x1200_0000 | ((0x80 - 8) >> 2) | ((0x100 >> 2) << 7);
    hint_then_wait(&mut pipeline, &mut state, 8, hint_b, 100);
    state.pc = 0x40;
    let br = 0x3200_0000 | (((0x100 - 0x40) >> 2) << 7);
    let insn = decode(br).expect("br");
    let slot = pipeline.plan(&state, br, &insn);
    pipeline.retire(slot, Some(0x100));
    assert_eq!(
        pipeline.next_issue(),
        slot.cycle + 18,
        "hint A was replaced"
    );
}

#[test]
fn a_channel_access_after_a_park_is_not_retried() {
    let mut state = SpuState::new();
    let mut pipeline = SpuPipeline::new();
    let insn = decode(RCHCNT).expect("rchcnt");
    let slot = pipeline.plan(&state, RCHCNT, &insn);
    pipeline.retire(slot, None);
    pipeline.resume_after_park();
    state.pc = 4;
    let again = pipeline.plan(&state, RCHCNT, &insn);
    assert_eq!(again.cycle, 1);
}
