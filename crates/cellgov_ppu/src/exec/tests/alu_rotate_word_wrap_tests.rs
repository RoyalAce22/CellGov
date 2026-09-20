//! Wrapped rotate-word mask behavior across both halves of a PPU register.

use super::*;

#[test]
fn rlwinm_wrapped_mask_writes_both_word_copies_and_sets_cr0_from_the_full_result() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x8000_0001);
    exec_no_mem(
        &PpuInstruction::Rlwinm {
            ra: 5,
            rs: 3,
            sh: 0,
            mb: 28,
            me: 3,
            rc: true,
        },
        &mut state,
    );
    assert_eq!(state.gpr[5], 0x8000_0001_8000_0001);
    assert_eq!(state.cr_field(0), 0b1000);
}

#[test]
fn rlwnm_wrapped_mask_writes_both_word_copies() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x8000_0001);
    state.set_gpr(4, 0);
    exec_no_mem(
        &PpuInstruction::Rlwnm {
            ra: 5,
            rs: 3,
            rb: 4,
            mb: 28,
            me: 3,
            rc: false,
        },
        &mut state,
    );
    assert_eq!(state.gpr[5], 0x8000_0001_8000_0001);
}

#[test]
fn rlwimi_wrapped_mask_replaces_the_high_word_and_merges_the_low_word() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x8000_0001);
    state.set_gpr(5, 0x1111_1111_2222_2222);
    exec_no_mem(
        &PpuInstruction::Rlwimi {
            ra: 5,
            rs: 3,
            sh: 0,
            mb: 28,
            me: 3,
            rc: false,
        },
        &mut state,
    );
    assert_eq!(state.gpr[5], 0x8000_0001_8222_2221);
}
