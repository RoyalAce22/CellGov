//! Which PPE thread has a sibling created before it: the later of two
//! runnable threads, and neither when a thread runs alone.

use crate::runtime::state::Runtime;
use cellgov_exec::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

#[test]
fn only_the_later_of_two_ppe_threads_has_an_older_sibling() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(1), 10_000);
    let mut unit = || {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(id, vec![FakeOp::LoadImm(1), FakeOp::End]).as_ppe_thread()
        })
    };
    let ids = [unit(), unit()];
    let older = |id, runnable: &[_]| rt.ppe_siblings(id, runnable).2;
    assert!(!older(ids[0], &ids));
    assert!(older(ids[1], &ids));
    assert!(!older(ids[1], &ids[1..]));
}
