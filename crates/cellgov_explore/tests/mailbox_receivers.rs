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
/// Each receiver idles until the sender queues both messages, so
/// neither one finds the queue empty and every unit finishes. Which
/// receiver takes which message is the only thing the schedule
/// decides, and each one writes what it took to its own address.
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

/// A relation that called the two receivers independent would prune
/// these alternates, so the run would answer for a schedule space it
/// never entered.
#[test]
fn no_point_between_the_two_receivers_prunes() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    let between: Vec<usize> = log
        .branching_points()
        .filter(|point| {
            let (chosen, others) = (point.chosen, &point.runnable);
            (chosen == FIRST && others.contains(&SECOND))
                || (chosen == SECOND && others.contains(&FIRST))
        })
        .map(|point| point.step)
        .collect();
    assert!(
        !between.is_empty(),
        "the run holds at least one point where both receivers were runnable",
    );

    let result = explore_window(workload, &ExplorationConfig::default());
    for step in between {
        assert!(
            result
                .schedules
                .iter()
                .any(|record| record.branch_step == step
                    && (record.alternate_choice == FIRST || record.alternate_choice == SECOND)),
            "the other receiver at step {step} was pruned rather than replayed",
        );
    }
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
