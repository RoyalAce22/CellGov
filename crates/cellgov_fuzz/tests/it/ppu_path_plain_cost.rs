//! The plain path knows its cost only when the pricing run executed the
//! instructions the plain run executed.

use cellgov_fuzz::ppu_paths::{first_path_divergence, run_all_paths, PpuExecutionPath};
use cellgov_ppu::state::PpuState;

const DATA_BASE: u64 = 0x1000_0000;

/// `ori r0,r0,0`.
const NOP: u32 = 24 << 26;

/// `mtctr r4`: mtspr with SPR 9 in its split field.
const MTCTR_R4: u32 = 0x7c89_03a6;

/// `bctr`: bcctr with BO 20.
const BCTR: u32 = 0x4e80_0420;

fn addi(rt: u32, ra: u32, imm: u16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | u32::from(imm)
}

fn data_state() -> PpuState {
    let mut state = PpuState::new();
    state.set_gpr(4, DATA_BASE);
    state
}

#[test]
fn a_branch_into_decodable_data_leaves_the_plain_cost_unknown_and_no_cost_divergence() {
    let words = [MTCTR_R4, BCTR, NOP, NOP, NOP, NOP];
    let data = NOP.to_be_bytes().repeat(16);
    let runs = run_all_paths(&words, &data_state(), &data).expect("the case runs");
    assert_eq!(runs.len(), 4);
    assert_eq!(runs[0].path, PpuExecutionPath::Plain);
    assert_eq!(runs[0].retired, 2);
    assert_eq!(runs[0].executed_pcs, vec![0, 4]);
    assert_eq!(runs[0].consumed_cost, None);
    for run in &runs[1..] {
        assert_eq!(run.retired, 6, "{:?} runs the data words", run.path);
        assert!(run.consumed_cost.is_some(), "{:?} is priced", run.path);
    }
    let divergence = first_path_divergence(&runs).expect("the retired counts differ");
    assert_eq!(
        (divergence.left, divergence.right),
        (PpuExecutionPath::Plain, PpuExecutionPath::Forwarded)
    );
    assert!(divergence.retired_differs);
    assert!(!divergence.cost_differs);
}

#[test]
fn a_program_that_stays_in_code_has_a_known_plain_cost_equal_to_every_path() {
    let words = [addi(3, 0, 1), addi(3, 3, 2), NOP, NOP];
    let runs = run_all_paths(&words, &data_state(), &[0; 64]).expect("the case runs");
    assert_eq!(runs.len(), 4);
    assert_eq!(first_path_divergence(&runs), None);
    let plain = runs[0].consumed_cost;
    assert!(plain.is_some_and(|cost| cost > 0), "plain cost {plain:?}");
    for run in &runs[1..] {
        assert_eq!(run.consumed_cost, plain, "{:?}", run.path);
    }
}
