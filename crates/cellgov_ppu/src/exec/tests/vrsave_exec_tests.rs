//! VRSAVE through `mfvrsave` and `mtvrsave`: a read before any write is
//! legal and returns the reset value, and a write moves the state hash.

use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;

/// [AltiVec-PEM p:2-6 s:2.3.3] VRSAVE resets to 0.
/// [PPC-Book1 p:82 s:3.3.13] mfspr of a 32-bit SPR zeroes RT[0:31].
#[test]
fn a_cold_mfvrsave_returns_zero_in_rt() {
    let mut s = PpuState::new();
    s.set_gpr(5, u64::MAX);
    let verdict = exec_no_mem(&PpuInstruction::Mfvrsave { rt: 5 }, &mut s);
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.mfvrsave_executed, 1);
}

#[test]
fn mtvrsave_moves_the_hash_by_the_vrsave_lane_alone() {
    let mut s = PpuState::new();
    s.set_gpr(3, 0xFFFF_FFFF_8000_0001);
    let before = s.state_hash();
    exec_no_mem(&PpuInstruction::Mtvrsave { rs: 3 }, &mut s);
    let mut expected = PpuState::new();
    expected.set_gpr(3, 0xFFFF_FFFF_8000_0001);
    expected.set_vrsave(0x8000_0001);
    assert_ne!(s.state_hash(), before);
    assert_eq!(s.state_hash(), expected.state_hash());
}
