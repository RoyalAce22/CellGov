//! A `Consumed` placeholder handed to `execute` breaks an internal
//! invariant: debug builds panic, release builds stop with an internal
//! fault.

use super::*;
use crate::exec::test_support::exec_no_mem;

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "a Consumed placeholder reached execute")]
fn a_consumed_placeholder_trips_the_debug_invariant() {
    exec_no_mem(&PpuInstruction::Consumed, &mut PpuState::new());
}

#[test]
#[cfg(not(debug_assertions))]
fn a_consumed_placeholder_is_refused_in_release() {
    let mut s = PpuState::new();
    let verdict = exec_no_mem(&PpuInstruction::Consumed, &mut s);
    assert!(matches!(
        verdict,
        ExecuteVerdict::Fault(crate::exec::PpuFault::Internal(".consumed"))
    ));
    assert_eq!(s.pc, 0, "the refusal leaves the state as it was");
}
