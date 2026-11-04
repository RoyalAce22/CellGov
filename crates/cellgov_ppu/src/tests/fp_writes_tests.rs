//! Every primary-63 and primary-59 arm writes its destination: none
//! retires as a silent no-op.

use strum::IntoEnumIterator;

use super::*;
use crate::observation::PpuArchitecturalState;

/// FPR 0 holds a value no arm produces from the operands below; FPRs 1,
/// 2 and 3 hold FRA, FRB and FRC. FX is set so `mcrfs` and `mtfsb0`
/// have a bit to move.
fn start() -> PpuState {
    let mut s = PpuState::new();
    s.set_fpr(0, 0x1234_5678_9ABC_DEF0);
    s.set_fpr(1, 1.5f64.to_bits());
    s.set_fpr(2, 2.5f64.to_bits());
    s.set_fpr(3, (-3.25f64).to_bits());
    s.set_fpscr(0x8000_0000);
    s
}

#[test]
fn every_primary_63_arm_changes_the_state() {
    let mut ops = 0;
    for op in Fp63Op::iter() {
        // mtfsb1 needs a clear bit (BT 3, OX); mtfsf needs a nonzero
        // FLM (bits 7:10 from the frt field select fields 0-3).
        let frt = match op {
            Fp63Op::Mtfsb1 => 3,
            Fp63Op::Mtfsf => 0xF,
            _ => 0,
        };
        let mut s = start();
        let before = PpuArchitecturalState::capture(&s);
        assert_eq!(
            execute_fp63(&mut s, op, frt, 1, 2, 3),
            ExecuteVerdict::Continue
        );
        assert_ne!(PpuArchitecturalState::capture(&s), before, "{op:?}");
        ops += 1;
    }
    assert_eq!(ops, 29);
}

#[test]
fn every_primary_59_arm_changes_the_state() {
    let mut ops = 0;
    for op in Fp59Op::iter() {
        let mut s = start();
        let before = PpuArchitecturalState::capture(&s);
        assert_eq!(
            execute_fp59(&mut s, op, 0, 1, 2, 3),
            ExecuteVerdict::Continue
        );
        assert_ne!(PpuArchitecturalState::capture(&s), before, "{op:?}");
        ops += 1;
    }
    assert_eq!(ops, 10);
}
