use super::seed_process_start_registers;

#[test]
fn a_primary_entry_without_a_termination_function_seeds_r7_zero() {
    let mut state = cellgov_ppu::state::PpuState::new();
    seed_process_start_registers(&mut state, None, 0x10_0000, 0x1_0000);
    assert_eq!(state.gpr[7], 0);
}
