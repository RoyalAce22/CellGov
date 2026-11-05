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

/// [PPC-Book1 p:87 s:4.2.2] VX is the OR of the invalid-operation bits and FEX the OR of each exception bit with its enable.
/// In the ppu_mcrfs_ox console capture, mtfsf 0xFF from 0x9F00_0000 reads back 0xBF00_0000.
#[test]
fn a_seeded_fpscr_takes_the_summaries_an_fpscr_write_gives() {
    let mut state = PpuState::new();
    for (seed, want) in [
        (0x9F00_0000, 0xBF00_0000),
        (0x6000_0000, 0x0000_0000),
        (0x0400_0010, 0x4400_0010),
    ] {
        let mut snapshot = PpuStateSnapshot::zero();
        snapshot.fpscr = seed;
        snapshot.apply(&mut state);
        assert_eq!(state.fpscr(), want, "seed {seed:#010x}");
    }
}
