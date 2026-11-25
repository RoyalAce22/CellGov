//! The commit of an answered hypercall queues r3, the register writes and
//! the CR fields for the caller's next step. The `SyncState` checkpoint
//! that the commit writes falls before that step.

use cellgov_event::UnitId;
use cellgov_exec::{CompareFields, CompareResult};
use cellgov_trace::{HashCheckpointKind, TraceReader, TraceRecord};

use super::{runtime, step};
use crate::runtime::Runtime;
use crate::scheduler::RoundRobinScheduler;

/// The hash of the last `SyncState` checkpoint in `rt`'s trace.
fn last_sync_checkpoint(rt: &Runtime) -> u64 {
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::SyncState,
                hash,
            } => Some(hash.raw()),
            _ => None,
        })
        .last()
        .expect("the commit writes a SyncState checkpoint")
}

#[test]
fn the_checkpoint_after_an_answered_hypercall_separates_its_pending_inputs() {
    let (mut rt, _) = runtime(74, [0; 8]);
    step(&mut rt).expect("the hypercall step");
    let checkpoint = last_sync_checkpoint(&rt);
    assert_eq!(checkpoint, rt.sync_state_hash());
    assert_eq!(checkpoint, rt.sync_state_hash_from_scratch());

    let unit = UnitId::new(0);
    let mut drained = rt.registry().clone();
    assert_eq!(drained.drain_syscall_return(unit), Some(0));
    let regs: Vec<u8> = drained
        .drain_register_writes(unit)
        .iter()
        .map(|&(reg, _)| reg)
        .collect();
    assert_eq!(regs, [4, 5, 6, 7, 8, 9, 10, 0, 11, 12]);
    let mut cr = [None; 8];
    cr[0] = Some(CompareResult::Equal);
    cr[1] = Some(CompareResult::Less);
    cr[7] = Some(CompareResult::Greater);
    assert_eq!(drained.drain_compare_fields(unit), Some(CompareFields(cr)));

    *rt.registry_mut() = drained;
    assert_ne!(rt.sync_state_hash(), checkpoint);
    assert_ne!(rt.sync_state_hash_from_scratch(), checkpoint);
}

#[test]
fn a_restore_to_the_checkpoint_gives_the_callers_next_step_its_pending_inputs() {
    let (mut rt, received) = runtime(74, [0; 8]);
    step(&mut rt).expect("the hypercall step");
    let checkpoint = rt.sync_state_hash();
    let snap = rt.snapshot();
    step(&mut rt).expect("the caller's next step");
    let first = received.borrow_mut().take();
    assert!(first.is_some(), "the caller's next step ran");

    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());
    assert_eq!(rt.sync_state_hash(), checkpoint);
    step(&mut rt).expect("the caller's next step after the restore");
    assert_eq!(received.borrow_mut().take(), first);
}
