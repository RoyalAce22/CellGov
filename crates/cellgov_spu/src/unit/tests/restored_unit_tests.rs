//! A unit built from a snapshot is the unit `new` then `restore` builds.

use crate::state::{SpuState, SPU_REG_COUNT};
use crate::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionUnit, UnitStatus};

/// A unit whose registers, local store, PC and status all differ from a
/// new unit's.
fn varied_unit() -> SpuExecutionUnit {
    let mut unit = SpuExecutionUnit::new(UnitId::new(5));
    let state = unit.state_mut();
    for k in 0..SPU_REG_COUNT {
        state.set_reg(k, [k as u8 ^ 0x5a; 16]);
    }
    state.ls[0x1230] = 0x77;
    state.pc = 0x88;
    unit.status = UnitStatus::Finished;
    unit
}

#[test]
fn a_restored_unit_equals_a_new_unit_after_restore() {
    let snapshot = varied_unit().snapshot();
    let mut expected = SpuExecutionUnit::new(UnitId::new(9));
    expected.restore(snapshot.clone());
    let restored = SpuExecutionUnit::restored(UnitId::new(9), snapshot);
    assert_eq!(restored.snapshot(), expected.snapshot());
    assert_eq!(restored.id, expected.id);
    assert_eq!(restored.retirement_counter, expected.retirement_counter);
    assert_eq!(restored.full_state_window, expected.full_state_window);
    assert!(restored.barriers.is_empty());
    assert!(restored.per_step_hashes.is_empty());
    assert!(restored.per_step_full_states.is_empty());
}

#[test]
fn a_new_unit_is_runnable_with_a_new_state() {
    let unit = SpuExecutionUnit::new(UnitId::new(2));
    assert_eq!(unit.status, UnitStatus::Runnable);
    assert_eq!(unit.stall, None);
    assert_eq!(*unit.state(), SpuState::new());
}

#[test]
fn into_state_hands_back_the_units_local_store() {
    let unit = varied_unit();
    let expected = unit.state().clone();
    let pointer = unit.state().ls.as_ptr();
    let state = unit.into_state();
    assert_eq!(state, expected);
    assert_eq!(state.ls.as_ptr(), pointer);
}
