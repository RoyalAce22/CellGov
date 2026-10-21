use super::*;
use crate::decode::decode;

/// Issue `words` at consecutive addresses from 0, the first as a branch
/// target, calling `between` before each with its index; return each
/// cycle.
fn issue_from_zero(words: &[u32], mut between: impl FnMut(usize, &mut SpuPipeline)) -> Vec<u64> {
    let mut pipeline = SpuPipeline::new();
    let mut state = SpuState::new();
    words
        .iter()
        .enumerate()
        .map(|(i, &raw)| {
            between(i, &mut pipeline);
            state.pc = 4 * i as u32;
            let insn = decode(raw).expect("an SPU instruction");
            let slot = pipeline.plan(&state, raw, &insn);
            pipeline.retire(slot, None);
            slot.cycle
        })
        .collect()
}

/// The first instruction of the fourth fetch line from 0.
const FOURTH_LINE: usize = 3 * SPU_FETCH_LINE_BYTES as usize / 4;

const LNOP: u32 = 0x0020_0000;
const NOP: u32 = 0x4020_007F;
/// lqa $10,0x9000
const LQA: u32 = 0x3092_000A;

/// The extra cycles the first instruction of the fourth line takes past
/// the one before it.
fn fourth_line_delay(cycles: &[u64]) -> u64 {
    cycles[FOURTH_LINE] - cycles[FOURTH_LINE - 1] - 1
}

#[test]
fn a_run_of_any_local_store_instruction_runs_out_in_the_fourth_line() {
    for (name, raw) in [
        ("lqd $10,0($1)", 0x3400_008A),
        ("lqx $10,$1,$2", 0x3880_808A),
        ("lqa $10,0x9000", LQA),
        ("lqr $10,0", 0x3380_000A),
        ("stqd $11,0($1)", 0x2400_008B),
        ("stqx $11,$1,$2", 0x2880_808B),
        ("stqa $11,0x9000", 0x2092_000B),
        ("stqr $11,0", 0x2380_000B),
        ("hbr 0,$1", 0x3580_0080),
        ("hbra 0,0", 0x1000_0000),
        ("hbrr 0,0", 0x1200_0000),
    ] {
        let cycles = issue_from_zero(&[raw; FOURTH_LINE + 1], |_, _| {});
        assert_eq!(fourth_line_delay(&cycles), SPU_RUNOUT_CYCLES, "{name}");
    }
}

#[test]
fn a_run_of_lnop_leaves_prefetch_its_slots() {
    let cycles = issue_from_zero(&[LNOP; FOURTH_LINE + 1], |_, _| {});
    assert_eq!(fourth_line_delay(&cycles), 0);
}

#[test]
fn a_load_dual_issued_with_a_nop_takes_the_local_store_in_their_cycle() {
    let pairs: Vec<u32> = [NOP, LQA].repeat(FOURTH_LINE / 2 + 1);
    let cycles = issue_from_zero(&pairs, |_, _| {});
    assert_eq!(cycles[FOURTH_LINE - 1], cycles[FOURTH_LINE - 2], "a pair");
    assert_eq!(
        cycles[FOURTH_LINE] - cycles[FOURTH_LINE - 1] - 1,
        SPU_RUNOUT_CYCLES
    );
}

#[test]
fn an_interrupt_redirect_or_a_restart_fetches_three_lines_again() {
    let redirect = |i: usize, p: &mut SpuPipeline| {
        if i == FOURTH_LINE {
            p.redirect(4 * i as u32);
        }
    };
    let restart = |i: usize, p: &mut SpuPipeline| {
        if i == FOURTH_LINE {
            p.clear_hints();
        }
    };
    let words = [LQA; FOURTH_LINE + 1];
    // The redirect's refetch holds the issue; no runout adds to it.
    let after_redirect = issue_from_zero(&words, redirect);
    assert_eq!(
        after_redirect[FOURTH_LINE] - after_redirect[FOURTH_LINE - 1],
        branch_miss_cycles(4 * FOURTH_LINE as u32)
    );
    assert_eq!(fourth_line_delay(&issue_from_zero(&words, restart)), 0);
}
