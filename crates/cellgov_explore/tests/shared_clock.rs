//! Witness: guest time is a shared resource no footprint records.
//!
//! A step advances one global clock, and a DMA completion lands at the
//! first commit whose clock passes the completion tick. So a step that
//! touches no shared resource still decides where an in-flight
//! transfer lands relative to every later step.
//!
//! The workload below is schedule-sensitive for that reason, and
//! `StepFootprint` calls the pair that decides it independent. The
//! footprint carries no clock, so the relation cannot see it.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::execution::Execution;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::prescribed::PrescribedScheduler;
use cellgov_explore::util::{run_to_stall, StopReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{CountingUnit, DmaSubmitter, WritingUnit};
use cellgov_time::Budget;

const WRITER: UnitId = UnitId::new(1);
const COUNTER: UnitId = UnitId::new(2);
const STEP_CAP: usize = 400;
/// Stores unit 1 makes over the destination; the last one carries
/// `[WRITER_STEPS; 4]`.
const WRITER_STEPS: u64 = 3;
/// Steps unit 2 takes. Unit 2 has to outlast the other two units, so
/// that the rotation keeps offering it after they finish.
const COUNTER_STEPS: u64 = 40;
const TRANSFERRED: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).expect("destination range")
}

/// One destination range, three units, and a schedule choice over when
/// the transfer lands.
///
/// Unit 0 submits the transfer, then blocks until it completes. Budget
/// 2 and the workspace DMA latency place the completion between unit
/// 1's last two possible write positions. So one step of unit 2, run
/// earlier, decides whether the transfer or the write lands last.
fn workload() -> Runtime {
    let src = ByteRange::new(GuestAddr::new(0), 4).expect("source range");
    let dst = destination();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(2), STEP_CAP);
    rt.register_unit_with(|id| DmaSubmitter::new(id, src, dst, TRANSFERRED.to_vec()));
    let writer = rt.register_unit_with(|id| WritingUnit::new(id, WRITER_STEPS, dst));
    let counter = rt.register_unit_with(|id| CountingUnit::new(id, COUNTER_STEPS));
    // The override list and the independence check both name units by
    // id, so a registration inserted above would retarget them.
    assert_eq!(writer, WRITER, "registration order moved the writer");
    assert_eq!(counter, COUNTER, "registration order moved the counter");
    rt
}

/// Final committed-memory hash and destination bytes of the run that
/// takes `overrides`.
fn run_with(overrides: Vec<Option<UnitId>>) -> (u64, Vec<u8>) {
    let mut rt = workload();
    rt.set_scheduler(PrescribedScheduler::new(overrides));
    assert_eq!(
        run_to_stall(&mut rt, STEP_CAP),
        StopReason::Stalled,
        "the workload runs itself out under every schedule the witness takes",
    );
    let landed = rt
        .memory()
        .read(destination())
        .expect("destination mapped")
        .to_vec();
    (rt.committed_memory_hash(), landed)
}

#[test]
fn a_counting_step_run_first_changes_committed_memory() {
    let (default_hash, default_bytes) = run_with(Vec::new());
    let (moved_hash, moved_bytes) = run_with(vec![None, Some(COUNTER)]);
    assert_ne!(default_hash, moved_hash);
    // The two byte checks hold the divergence to the transfer. A hash
    // that moved for any other reason fails them.
    assert_eq!(
        default_bytes, TRANSFERRED,
        "by default the transfer lands after unit 1's last store",
    );
    assert_eq!(
        moved_bytes, [WRITER_STEPS as u8; 4],
        "one counting step run earlier puts the transfer before that store",
    );
}

#[test]
fn the_relation_calls_the_pair_that_decides_it_independent() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(
        stop,
        StopReason::Stalled,
        "a prefix of the schedule answers for no pair",
    );
    let execution = Execution::from_log(&log);
    assert!(
        execution.units_independent(WRITER, COUNTER),
        "a counting step records no shared access, so nothing holds it against the writer",
    );
}

#[test]
fn the_verdict_reads_schedule_stable() {
    let result = explore_window(workload, &ExplorationConfig::default());
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::ScheduleStable,
        "the pruned alternate is the one that diverges",
    );
    assert!(!result.bounds_hit, "no bound withdraws the claim");
    assert_eq!(
        result.baseline_stop,
        StopReason::Stalled,
        "the baseline hash covers the whole workload",
    );
    // The same verdict follows from a baseline that holds no branching
    // point, so the prune needs a witness of its own.
    assert!(
        result.schedules_pruned > 0,
        "the verdict rests on a prune, so at least one alternate must be pruned",
    );
}
