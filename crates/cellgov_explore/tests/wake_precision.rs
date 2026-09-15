//! What the narrowed wake rule buys, measured.
//!
//! A wake enables only the unit it names. The rule it replaced paired
//! any wake with any wait, so one wake made every waiting unit in the
//! execution a conflict partner and the pruning the rest of the
//! relation earns was thrown away at that pair.
//!
//! The workload here is the shape that rule cost most: several units
//! that wait, and one waker whose every wake names just one of them.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_effects::{Effect, WaitTarget};
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{explore_optimal, ExplorationConfig, StepFootprint};
use cellgov_mem::GuestMemory;
use cellgov_sync::{BarrierId, MailboxId};
use cellgov_time::Budget;

/// One waker that wakes each waiter in turn, and `waiters` units that
/// each wait on their own barrier and then store to their own address.
///
/// This is the shape the rule this replaced cost most: `n` wake steps
/// against `n` wait steps made `n * n` conflicting pairs where only
/// `n` of them are real.
fn one_waker_many_waiters(waiters: u64) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 400);
    // Waiters first, so the search's default descent parks each of
    // them before it reaches the waker. An order that wakes a waiter
    // before it parks wastes that wake, and the waiter never runs
    // again.
    for index in 0..waiters {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::Barrier { barrier: index },
                    FakeOp::LoadImm(0xB0 + index as u32),
                    FakeOp::SharedStore {
                        addr: index * 8,
                        len: 4,
                    },
                    FakeOp::End,
                ],
            )
        });
    }
    let waker: Vec<FakeOp> = (0..waiters)
        .map(|index| FakeOp::Wake { unit: index })
        .chain([FakeOp::End])
        .collect();
    rt.register_unit_with(|id| FakeIsaUnit::new(id, waker.clone()));
    rt
}

/// Executions the search runs, baseline included, and the class count
/// it reports.
fn measure(waiters: u64) -> (usize, Option<usize>) {
    let result = explore_optimal(
        || one_waker_many_waiters(waiters),
        &ExplorationConfig {
            max_schedules: 100_000,
            max_steps_per_run: 10_000,
        },
    );
    (result.schedules.len() + 1, result.classes_explored)
}

/// Each wake depends on the one wait it enables and on nothing else,
/// so the search runs one execution per equivalence class.
///
/// Each waiter decides one thing on its own: whether its wake lands
/// before it parks, which wastes the wake and leaves it parked for
/// good. Those decisions are independent, so the workload holds two to
/// the power of the waiters. The counts below are that number twice:
/// executions on the left, the classes they cover on the right.
///
/// Covering every class is what the narrowing buys. The rule this
/// replaced paired every wake with every wait. Its false dependencies
/// produced races whose reversals no state can reach, and the search
/// drops those branches: a single drop withdraws the class count on
/// the right.
#[test]
fn the_narrowed_rule_covers_every_class_it_explores() {
    let measured: Vec<(usize, Option<usize>)> = (1..=4).map(measure).collect();
    assert_eq!(
        measured,
        vec![(2, Some(2)), (4, Some(4)), (8, Some(8)), (16, Some(16))],
    );
}

/// The rule this replaced, restated: any wake against any wait.
fn blanket(wake: &StepFootprint, wait: &StepFootprint) -> bool {
    let waits_on_anything = !wait.wait_mailboxes.is_empty()
        || !wait.wait_signals.is_empty()
        || !wait.wait_barriers.is_empty();
    !wake.wake_targets.is_empty() && waits_on_anything
}

/// The pairs the narrowing drops: a wake of one unit against a wait by
/// any other.
#[test]
fn the_narrowing_drops_every_pair_but_the_one_the_wake_names() {
    let woken = UnitId::new(1);
    let wake = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: woken,
        source: UnitId::new(0),
    }]);
    let mut dropped = 0;
    for raw in 1..=4 {
        let waiter = UnitId::new(raw);
        let wait = StepFootprint::from_effects(&[Effect::WaitOnEvent {
            target: WaitTarget::Barrier(BarrierId::new(raw)),
            source: waiter,
        }]);
        assert!(
            blanket(&wake, &wait),
            "the rule this replaced paired every one of these",
        );
        if waiter == woken {
            assert!(wake.conflicts(&wait), "the pair the wake names survives");
        } else {
            assert!(!wake.conflicts(&wait));
            dropped += 1;
        }
    }
    assert_eq!(dropped, 3, "one waiter of four is the one the wake names");
}

/// A receive attempt is the other step that parks its own unit: the
/// commit pipeline blocks the source when the mailbox comes back
/// empty, and nothing auto-wakes it on a later send. So the wake that
/// names the receiver decides whether it ends runnable or parked with
/// no wake source left, and the pair is order-dependent even though
/// the wake touches no mailbox.
#[test]
fn a_wake_conflicts_with_the_receive_attempt_that_parks_the_same_unit() {
    let receiver = UnitId::new(1);
    let wake = StepFootprint::from_effects(&[Effect::WakeUnit {
        target: receiver,
        source: UnitId::new(0),
    }]);
    let receive = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(7),
        source: receiver,
    }]);
    assert!(wake.conflicts(&receive));
    assert!(
        receive.conflicts(&wake),
        "the pair conflicts from either side",
    );

    let other = StepFootprint::from_effects(&[Effect::MailboxReceiveAttempt {
        mailbox: MailboxId::new(7),
        source: UnitId::new(2),
    }]);
    assert!(
        !wake.conflicts(&other),
        "the wake reaches only the unit it names",
    );
}
