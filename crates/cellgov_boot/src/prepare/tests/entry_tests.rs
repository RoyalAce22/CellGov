use super::{seed_process_start_registers, ProcessStart};
use crate::prx::TlsArea;

fn start() -> ProcessStart {
    ProcessStart {
        entry: 0x1_0000,
        tls: TlsArea::above(0x10a5_7708).expect("in range"),
    }
}

#[test]
fn a_primary_entry_without_a_termination_function_seeds_r7_zero() {
    let mut state = cellgov_ppu::state::PpuState::new();
    seed_process_start_registers(&mut state, None, 0x10_0000, start());
    assert_eq!(state.gpr[7], 0);
}

#[test]
fn the_primary_entry_r13_points_into_the_placed_tls_area() {
    let mut state = cellgov_ppu::state::PpuState::new();
    seed_process_start_registers(&mut state, None, 0x10_0000, start());
    assert_eq!(state.gpr[13], 0x10a6_0000 + 0x7030);
}
