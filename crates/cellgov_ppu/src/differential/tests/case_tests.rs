//! Tests for applying a register snapshot to a PPU state.

use super::*;

/// In the ppu_fpscr_ni_write console capture, no write leaves NI or bit 20 set.
#[test]
fn a_seeded_fpscr_drops_the_bits_the_ppe_does_not_implement() {
    let mut snapshot = PpuStateSnapshot::zero();
    snapshot.fpscr = 0x0000_0807;
    let mut state = PpuState::new();
    snapshot.apply(&mut state);
    assert_eq!(state.fpscr(), 0x0000_0003);
}
