//! Tests PPU optimization consistency across paths.

use cellgov_exec::YieldReason;
use cellgov_fuzz::ppu_paths::{first_path_divergence, run_all_paths, PpuExecutionPath};
use cellgov_ppu::state::PpuState;

const DATA_BASE: u64 = 0x1000_0000;

fn stw(rs: u32, ra: u32, offset: u16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

fn lwz(rt: u32, ra: u32, offset: u16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(offset)
}

fn sth(rs: u32, ra: u32, offset: u16) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

fn stwcx(rs: u32, ra: u32, rb: u32) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (150 << 1) | 1
}

fn li(rt: u32, value: u16) -> u32 {
    (14 << 26) | (rt << 21) | u32::from(value)
}

fn mftb(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (12 << 16) | (8 << 11) | (371 << 1)
}

#[test]
fn store_forwarding_matches_flush_per_step() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x1122_3344);
    state.set_gpr(4, DATA_BASE);

    let runs = run_all_paths(&[stw(3, 4, 0), lwz(5, 4, 0)], &state, &[0; 64])
        .expect("forwarding case must run");

    assert_eq!(runs.len(), 4);
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs
        .iter()
        .all(|run| run.observation.state.gpr[5] == 0x1122_3344));
}

#[test]
fn fused_li_store_matches_the_other_paths() {
    let mut state = PpuState::new();
    state.set_gpr(4, DATA_BASE);

    let runs = run_all_paths(&[li(3, 7), stw(3, 4, 0)], &state, &[0; 64])
        .expect("fused store case must run");

    let code = [li(3, 7), stw(3, 4, 0)]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect::<Vec<_>>();
    assert!(!matches!(
        cellgov_ppu::shadow::PredecodedShadow::build_quickened(0, &code).get(4),
        Some(cellgov_ppu::instruction::PpuInstruction::Consumed)
    ));
    assert!(matches!(
        cellgov_ppu::shadow::PredecodedShadow::build(0, &code).get(4),
        Some(cellgov_ppu::instruction::PpuInstruction::Consumed)
    ));

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert_eq!(
        runs.iter().map(|run| run.path).collect::<Vec<_>>(),
        [
            PpuExecutionPath::Plain,
            PpuExecutionPath::Forwarded,
            PpuExecutionPath::Quickened,
            PpuExecutionPath::Fused,
        ]
    );
    assert!(runs
        .iter()
        .all(|run| run.observation.memory[..4] == 7u32.to_be_bytes()));
}

#[test]
fn a_clock_read_is_emitted_once_on_every_path() {
    let state = PpuState::new();

    let runs = run_all_paths(&[mftb(3)], &state, &[0; 64]).expect("clock-read case must run");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs.iter().all(|run| {
        run.observation
            .committed_effects
            .iter()
            .filter(|effect| matches!(effect, cellgov_effects::Effect::ClockRead { .. }))
            .count()
            == 1
    }));
}

#[test]
fn a_seeded_forwarded_state_difference_names_the_first_path_pair_and_component() {
    let state = PpuState::new();
    let mut runs = run_all_paths(&[li(3, 1)], &state, &[0; 64]).expect("seeded case must run");
    runs[1].observation.state.gpr[3] = 2;

    let divergence = first_path_divergence(&runs).expect("seeded leak must diverge");

    assert_eq!(divergence.left, PpuExecutionPath::Plain);
    assert_eq!(divergence.right, PpuExecutionPath::Forwarded);
    assert!(divergence
        .observation
        .contains(&cellgov_ppu::observation::PpuObservationComponent::State));
}

#[test]
fn a_seeded_quickened_memory_difference_names_the_path_and_component() {
    let state = PpuState::new();
    let mut runs = run_all_paths(&[li(3, 1)], &state, &[0; 64]).expect("seeded case must run");
    runs[2].observation.memory[0] = 1;

    let divergence = first_path_divergence(&runs).expect("seeded invalidation leak must diverge");

    assert_eq!(divergence.left, PpuExecutionPath::Plain);
    assert_eq!(divergence.right, PpuExecutionPath::Quickened);
    assert!(divergence
        .observation
        .contains(&cellgov_ppu::observation::PpuObservationComponent::Memory));
}

#[test]
fn a_seeded_fused_state_difference_names_the_path_and_component() {
    let state = PpuState::new();
    let mut runs = run_all_paths(&[li(3, 1)], &state, &[0; 64]).expect("seeded case must run");
    runs[3].observation.state.gpr[3] = 2;

    let divergence = first_path_divergence(&runs).expect("seeded fusion leak must diverge");

    assert_eq!(divergence.left, PpuExecutionPath::Plain);
    assert_eq!(divergence.right, PpuExecutionPath::Fused);
    assert!(divergence
        .observation
        .contains(&cellgov_ppu::observation::PpuObservationComponent::State));
}

#[test]
fn a_seeded_stop_difference_names_the_path_and_stop() {
    let state = PpuState::new();
    let mut runs = run_all_paths(&[li(3, 1)], &state, &[0; 64]).expect("seeded case must run");
    runs[1].stop.reason = YieldReason::Fault;

    let divergence = first_path_divergence(&runs).expect("seeded fault leak must diverge");

    assert_eq!(divergence.left, PpuExecutionPath::Plain);
    assert_eq!(divergence.right, PpuExecutionPath::Forwarded);
    assert!(divergence.stop_differs);
}

#[test]
fn overlapping_writes_keep_program_order_on_every_path() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x1122_3344);
    state.set_gpr(4, DATA_BASE);
    state.set_gpr(5, 0xaabb);

    let runs = run_all_paths(&[stw(3, 4, 0), sth(5, 4, 1)], &state, &[0; 64])
        .expect("overlap case must run");

    assert!(first_path_divergence(&runs).is_none());
    assert!(runs
        .iter()
        .all(|run| run.observation.memory[..4] == [0x11, 0xaa, 0xbb, 0x44]));
}

#[test]
fn a_conditional_store_retires_the_seeded_reservation_on_every_path() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x5566_7788);
    state.set_gpr(4, DATA_BASE);
    state.set_gpr(5, 0);
    state.set_reservation(Some(cellgov_sync::ReservedLine::containing(DATA_BASE)));

    let runs = run_all_paths(&[stwcx(3, 4, 5)], &state, &[0; 64])
        .expect("conditional store case must run");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs
        .iter()
        .all(|run| run.observation.reservations.is_empty()));
    assert!(runs
        .iter()
        .all(|run| run.observation.memory[..4] == 0x5566_7788u32.to_be_bytes()));
}

#[test]
fn a_later_fault_discards_the_whole_logical_batch_on_every_path() {
    let li_r3_one = li(3, 1);
    let lwz_r4_unmapped = lwz(4, 6, 0);
    let mut state = PpuState::new();
    state.set_gpr(6, 0x2000_0000);

    let runs = run_all_paths(&[li_r3_one, lwz_r4_unmapped], &state, &[0; 64])
        .expect("fault case must run");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs.iter().all(|run| run.observation.state.gpr[3] == 0));
    assert!(runs.iter().all(|run| run.observation.fault_discarded));
}

#[test]
fn a_later_fault_restores_the_seeded_reservation_on_every_path() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x5566_7788);
    state.set_gpr(4, DATA_BASE);
    state.set_gpr(5, 0);
    state.set_gpr(6, 0x2000_0000);
    state.set_reservation(Some(cellgov_sync::ReservedLine::containing(DATA_BASE)));

    let runs = run_all_paths(&[stwcx(3, 4, 5), lwz(7, 6, 0)], &state, &[0; 64])
        .expect("reservation rollback case must run");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    assert!(runs.iter().all(|run| {
        run.observation.reservations
            == vec![(
                cellgov_event::UnitId::new(0),
                cellgov_sync::ReservedLine::containing(DATA_BASE),
            )]
    }));
}

#[test]
fn branch_entry_into_a_consumed_pair_is_a_visible_fused_path_divergence() {
    let branch_to_third_word = (18 << 26) | 8;
    let mut state = PpuState::new();
    state.set_gpr(3, 9);
    state.set_gpr(4, DATA_BASE);

    let words = [branch_to_third_word, li(3, 7), stw(3, 4, 0)];
    let code = words
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .collect::<Vec<_>>();
    let shadow = cellgov_ppu::shadow::PredecodedShadow::build(0, &code);
    assert!(matches!(
        shadow.get(8),
        Some(cellgov_ppu::instruction::PpuInstruction::Consumed)
    ));
    let runs = run_all_paths(&words, &state, &[0; 64]).expect("branch-entry case must run");
    assert_eq!(
        runs.iter()
            .map(|run| run.observation.memory[3])
            .collect::<Vec<_>>(),
        [9, 9, 9, 0]
    );
    let divergence = first_path_divergence(&[runs[1].clone(), runs[3].clone()])
        .expect("branch entry must distinguish the fused path");

    assert_eq!(divergence.left, PpuExecutionPath::Forwarded);
    assert_eq!(divergence.right, PpuExecutionPath::Fused);
    assert!(divergence
        .observation
        .contains(&cellgov_ppu::observation::PpuObservationComponent::Memory));
}
