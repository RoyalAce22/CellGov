use super::{classify, PpeTiming};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;

/// Retire `insns` from `pc` up as straight-line code, and return the
/// issue cycle of each op.
fn issue_all(timing: &mut PpeTiming, state: &PpuState, pc: u64, insns: &[I]) -> Vec<u64> {
    let mut cycles = Vec::new();
    for (i, insn) in insns.iter().enumerate() {
        let at = pc + 4 * i as u64;
        let (ops, n) = classify(insn, state);
        for op in &ops[..n] {
            cycles.push(timing.issue_cycle(at, op));
            timing.retire(at, op, at + 4);
        }
    }
    cycles
}

#[test]
fn a_fixed_point_op_and_a_load_at_an_aligned_pair_issue_together() {
    let mut timing = PpeTiming::new();
    let state = PpuState::new();
    let cycles = issue_all(
        &mut timing,
        &state,
        0x1000,
        &[
            I::Addi {
                rt: 14,
                ra: 14,
                imm: 1,
            },
            I::Ld {
                rt: 18,
                ra: 3,
                imm: 0,
            },
        ],
    );
    assert_eq!(cycles[0], cycles[1]);
}

#[test]
fn two_fixed_point_ops_at_an_aligned_pair_issue_a_cycle_apart() {
    let mut timing = PpeTiming::new();
    let state = PpuState::new();
    let cycles = issue_all(
        &mut timing,
        &state,
        0x1000,
        &[
            I::Addi {
                rt: 14,
                ra: 14,
                imm: 1,
            },
            I::Addi {
                rt: 15,
                ra: 15,
                imm: 1,
            },
        ],
    );
    assert_eq!(cycles[1], cycles[0] + 1);
}

#[test]
fn a_pair_split_across_an_eight_byte_boundary_issues_a_cycle_apart() {
    let mut timing = PpeTiming::new();
    let state = PpuState::new();
    let cycles = issue_all(
        &mut timing,
        &state,
        0x1004,
        &[
            I::Addi {
                rt: 14,
                ra: 14,
                imm: 1,
            },
            I::Ld {
                rt: 18,
                ra: 3,
                imm: 0,
            },
        ],
    );
    assert_eq!(cycles[1], cycles[0] + 1);
}

#[test]
fn a_dependent_add_waits_two_cycles() {
    let mut timing = PpeTiming::new();
    let state = PpuState::new();
    let add = I::Add {
        rt: 3,
        ra: 3,
        rb: 4,
        oe: false,
        rc: false,
    };
    let cycles = issue_all(&mut timing, &state, 0x1000, &[add, add]);
    assert_eq!(cycles[1], cycles[0] + 2);
}

#[test]
fn a_divide_of_a_33_bit_dividend_by_one_takes_the_handbook_s_38_cycles_and_the_refetch() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x1_2345_6789);
    state.set_gpr(15, 1);
    let divd = I::Divd {
        rt: 3,
        ra: 3,
        rb: 15,
        oe: false,
        rc: false,
    };
    let (ops, n) = classify(&divd, &state);
    assert_eq!(n, 1);
    assert_eq!(
        ops[0].dsts[0],
        (
            3,
            38 + cellgov_ps3_abi::hw::ppu_issue::PPU_DIVIDE_REFETCH_CYCLES as u16
        )
    );
}

#[test]
fn an_indexed_load_with_update_into_its_index_register_waits_for_the_miss() {
    use cellgov_ps3_abi::hw::ppu_issue as k;
    let mut timing = PpeTiming::new();
    let mut state = PpuState::new();
    state.set_gpr(3, 0x2000);
    state.set_gpr(4, 0x80);
    let cycles = issue_all(
        &mut timing,
        &state,
        0x1000,
        &[
            I::Lwzux {
                rt: 4,
                ra: 3,
                rb: 4,
            },
            I::Add {
                rt: 5,
                ra: 4,
                rb: 4,
                oe: false,
                rc: false,
            },
        ],
    );
    assert_eq!(cycles[1], cycles[0] + k::PPU_MEMORY_LOAD_USE_CYCLES);
}

#[test]
fn a_dependent_load_of_an_l1_line_waits_five_cycles() {
    let mut timing = PpeTiming::new();
    let mut state = PpuState::new();
    state.set_gpr(3, 0x2000);
    let ld = I::Ld {
        rt: 3,
        ra: 3,
        imm: 0,
    };
    // The first load fills the line; the next two chase through it.
    let cycles = issue_all(&mut timing, &state, 0x1000, &[ld, ld, ld]);
    assert_eq!(cycles[2], cycles[1] + 5);
}

#[test]
fn a_fused_pair_times_both_of_its_instructions() {
    let mut state = PpuState::new();
    state.set_gpr(1, 0x3000);
    let (_, n) = classify(
        &I::LwzCmpwi {
            rt: 3,
            ra_load: 1,
            offset: 0,
            bf: 0,
            cmp_imm: 0,
        },
        &state,
    );
    assert_eq!(n, 2);
    let (_, n) = classify(&I::Consumed, &state);
    assert_eq!(n, 0);
}
