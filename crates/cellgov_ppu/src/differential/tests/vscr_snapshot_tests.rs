//! The VSCR in the differential snapshot, its diff, and the observation.

use super::{run_case, CaseOutcome};
use crate::differential::{InstructionCase, MemorySnapshot, OracleSource, PpuStateSnapshot};
use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;

#[test]
fn a_snapshot_carries_the_vscr_both_ways() {
    let mut s = PpuState::new();
    s.set_vscr(0x0000_0001);
    let snap = PpuStateSnapshot::capture(&s);
    assert_eq!(snap.vscr, 0x0000_0001);

    let mut restored = PpuState::new();
    snap.apply(&mut restored);
    assert_eq!(restored.vscr(), 0x0000_0001);
    assert_eq!(PpuArchitecturalState::capture(&restored).vscr, 0x0000_0001);
}

#[test]
fn a_vscr_difference_is_reported() {
    let mut expected = PpuStateSnapshot::zero();
    expected.vscr = 0x0001_0000;
    let case = InstructionCase {
        label: "ori_nop_lying_vscr".to_string(),
        initial_state: PpuStateSnapshot::zero(),
        initial_memory: MemorySnapshot::empty(),
        raw_instruction: 0x6000_0000,
        expected_state: expected,
        expected_memory: MemorySnapshot::empty(),
        source: OracleSource::Manual,
    };
    match run_case(&case) {
        CaseOutcome::StateMismatch(diff) => {
            assert_eq!(diff.vscr, Some((0x0001_0000, 0)));
            assert!(diff.gpr.is_empty() && diff.vrsave.is_none());
        }
        other => panic!("expected StateMismatch, got {other:?}"),
    }
}
