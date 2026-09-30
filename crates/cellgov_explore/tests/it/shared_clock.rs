//! Witness: guest time decides where an in-flight transfer lands.
//!
//! A step advances one global clock, and a DMA completion lands at the
//! first commit whose clock passes the completion tick. So a step that
//! touches no shared resource still decides where an in-flight
//! transfer lands relative to every later step. The relation sees it:
//! a step taken while a transfer is in flight records that transfer's
//! ranges, and they conflict with another step's access to the bytes
//! it lands on.
//!
//! The clock's other two readers have their own witnesses:
//!
//! - `tests/clock_read.rs`, a `mftb`
//! - `tests/timer_deadline.rs`, a timer deadline

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
/// The last store carries `[WRITER_STEPS; 4]`, which the byte checks
/// read.
const WRITER_STEPS: u64 = 3;
/// Unit 2 has to outlast the other two, so the rotation keeps offering
/// it after they finish.
const COUNTER_STEPS: u64 = 40;
const TRANSFERRED: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

/// The latency this witness rests on: with budget 2 it puts the
/// completion between unit 1's last two write positions, so a change to
/// either moves the landing.
const LATENCY: u64 = 10;

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).expect("destination range")
}

/// One destination range, three units, and a schedule choice over when
/// the transfer lands: one step of unit 2, run earlier, decides whether
/// the transfer or unit 1's last store lands last (`LATENCY`).
fn workload() -> Runtime {
    assert_eq!(
        cellgov_core::DEFAULT_DMA_LATENCY_TICKS.raw(),
        LATENCY,
        "the workspace latency is what places the completion between the writer's \
         last two positions",
    );
    let src = ByteRange::new(GuestAddr::new(0), 4).expect("source range");
    let dst = destination();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(2), STEP_CAP);
    rt.register_unit_with(|id| DmaSubmitter::new(id, src, dst, TRANSFERRED.to_vec()));
    let writer = rt.register_unit_with(|id| WritingUnit::new(id, WRITER_STEPS, dst));
    let counter = rt.register_unit_with(|id| CountingUnit::new(id, COUNTER_STEPS));
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
fn the_relation_holds_the_pair_that_decides_it_apart() {
    let mut rt = workload();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(
        stop,
        StopReason::Stalled,
        "a prefix of the schedule answers for no pair",
    );
    let execution = Execution::from_log(&log);
    assert!(
        !execution.units_independent(WRITER, COUNTER),
        "the counter runs while the transfer is in flight and the writer stores over \
         where it lands, so the order matters",
    );
}

#[test]
fn the_verdict_reads_schedule_sensitive() {
    let result = explore_window(workload, &ExplorationConfig::default());
    assert_eq!(
        result.outcome,
        cellgov_explore::classify::OutcomeClass::ScheduleSensitive,
    );
    assert_eq!(
        result.baseline_stop,
        StopReason::Stalled,
        "the baseline runs the workload out",
    );

    // The search has to reach the memory
    // `a_counting_step_run_first_changes_committed_memory` writes by
    // hand.
    let (diverging, _) = run_with(vec![None, Some(COUNTER)]);
    assert_ne!(result.baseline_hash, diverging);
    assert!(
        result
            .schedules
            .iter()
            .any(|record| !record.truncated && record.memory_hash == diverging),
        "the search reached the schedule that lands the transfer first",
    );
}
