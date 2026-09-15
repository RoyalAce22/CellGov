//! Two units that receive from one mailbox race for the message.
//!
//! The commit pipeline hands a queued message to whichever receiver
//! commits first and blocks the other when the queue comes back empty.
//! So the pair is order-dependent with no send anywhere near it, and
//! the exploration has to replay it rather than prune it.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::classify::OutcomeClass;
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::execution::Execution;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::util::StopReason;
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;
use std::collections::BTreeSet;

const MAILBOX: u64 = 0;
const FIRST: UnitId = UnitId::new(1);
const SECOND: UnitId = UnitId::new(2);
const STEP_CAP: usize = 200;

/// Where each receiver stores the byte it took, so the final memory
/// names which one won.
const FIRST_ADDR: u64 = 0;
const SECOND_ADDR: u64 = 8;

/// One sender queues two distinct messages, then two receivers take
/// one each.
///
/// Each receiver idles through three steps first, so the default
/// schedule queues both messages before either receive attempt and
/// every unit finishes. Which receiver takes which message is the only
/// thing that schedule decides, and each one writes what it took to
/// its own address.
fn workload() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), STEP_CAP);
    let queue = rt.mailbox_registry_mut().register(4);
    assert_eq!(
        queue.raw(),
        MAILBOX,
        "the programs name their mailbox by raw id",
    );
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(0xAA),
                FakeOp::MailboxSend { mailbox: MAILBOX },
                FakeOp::LoadImm(0xBB),
                FakeOp::MailboxSend { mailbox: MAILBOX },
                FakeOp::End,
            ],
        )
    });
    for addr in [FIRST_ADDR, SECOND_ADDR] {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0),
                    FakeOp::LoadImm(0),
                    FakeOp::LoadImm(0),
                    FakeOp::MailboxRecv { mailbox: MAILBOX },
                    FakeOp::SharedStore { addr, len: 4 },
                    FakeOp::End,
                ],
            )
        });
    }
    let ids: Vec<UnitId> = rt.registry().ids().collect();
    assert_eq!(
        ids,
        vec![UnitId::new(0), FIRST, SECOND],
        "the overrides and the independence check name the receivers by id",
    );
    rt
}

#[test]
fn the_two_receivers_conflict() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled, "the workload runs itself out");
    assert!(
        !Execution::from_log(&log).units_independent(FIRST, SECOND),
        "one mailbox, two receive attempts: the order decides who takes the message",
    );
}

/// The search separates the two orders of the receive attempts.
///
/// The class count and the committed memories are what this costs. A
/// relation that called the pair independent reports no race between
/// them, so the search never owes their reversal, and both numbers
/// below shrink with it. One order deadlocks, and its memory is the
/// fifth: a receiver parked for good stored nothing.
#[test]
fn the_search_runs_both_orders_of_the_two_receivers() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    assert!(
        log.branching_points().any(|point| {
            let (chosen, others) = (point.chosen, &point.runnable);
            (chosen == FIRST && others.contains(&SECOND))
                || (chosen == SECOND && others.contains(&FIRST))
        }),
        "the run holds at least one point where both receivers were runnable",
    );

    let result = explore_window(workload, &ExplorationConfig::default());
    // Some orders leave a receiver parked on an empty queue with no
    // send left to wake it.
    assert_eq!(
        result.schedules_truncated, 0,
        "a deadlocked order is the end of its execution, not a cut prefix",
    );
    assert!(
        result
            .schedules
            .iter()
            .any(|record| record.stop == StopReason::Deadlocked),
        "an order that parks a receiver for good is one of the orders",
    );
    assert_eq!(
        result.classes_explored,
        Some(12),
        "one execution per equivalence class, deadlocked orders included",
    );
    let hashes: BTreeSet<u64> = std::iter::once(result.baseline_hash)
        .chain(result.schedules.iter().map(|record| record.memory_hash))
        .collect();
    assert_eq!(
        hashes.len(),
        5,
        "the committed memories the orders reach between them",
    );
}

#[test]
fn the_message_each_receiver_took_is_the_schedule_s_answer() {
    let result = explore_window(workload, &ExplorationConfig::default());
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert_eq!(
        result.outcome,
        OutcomeClass::ScheduleSensitive,
        "swapping the two receivers swaps which message each one stores",
    );
}
