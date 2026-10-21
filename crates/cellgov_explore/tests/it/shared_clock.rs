//! Witness: guest time decides where an in-flight transfer lands.
//!
//! Each unit keeps its own clock, and a DMA completion lands once no
//! runnable unit's clock is below its tick. So a unit whose clock lags
//! holds the landing back. A step that touches no shared resource then
//! still decides whether a faster unit's store past the completion tick
//! falls before or after the landing. The relation sees
//! it: a step taken while a transfer is in flight records that
//! transfer's ranges, and they conflict with another step's access to
//! the bytes it lands on.
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

const SUBMITTER: UnitId = UnitId::new(0);
const WRITER: UnitId = UnitId::new(1);
const COUNTER: UnitId = UnitId::new(2);
const STEP_CAP: usize = 400;
/// The last store carries `[WRITER_STEPS; 4]`, which the byte checks
/// read. It starts at tick 42, past the completion at tick 39.
const WRITER_STEPS: u64 = 7;
/// Unit 2's clock has to pass the completion tick for the transfer to
/// land.
const COUNTER_STEPS: u64 = 8;
const TRANSFERRED: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

/// The latency this witness rests on.
///
/// With budget 7 the submitter queues the transfer at tick 7, so it
/// completes at tick 39: between the starts of unit 1's last two stores,
/// at ticks 35 and 42. A change to either moves the landing.
const LATENCY: u64 = 32;
const BUDGET: u64 = 7;

fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).expect("destination range")
}

/// One destination range, three units, and a schedule choice over when
/// the transfer lands (`LATENCY`).
///
/// Unit 1's last store runs before or after unit 2's clock passes the
/// completion tick, and that order decides which of the two lands last.
fn workload() -> Runtime {
    assert_eq!(
        cellgov_core::DEFAULT_DMA_LATENCY_TICKS.raw(),
        LATENCY,
        "the workspace latency is what places the completion between the writer's \
         last two positions",
    );
    let src = ByteRange::new(GuestAddr::new(0), 4).expect("source range");
    let dst = destination();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(BUDGET), STEP_CAP);
    let submitter =
        rt.register_unit_with(|id| DmaSubmitter::new(id, src, dst, TRANSFERRED.to_vec()));
    assert_eq!(
        submitter, SUBMITTER,
        "registration order moved the submitter"
    );
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

/// The schedule that queues the transfer, then runs every store of
/// unit 1 before unit 2 takes a step.
///
/// The submitter goes first so the transfer is in flight under each
/// store. Without it every store runs before the enqueue, and the
/// transfer lands last whatever the counter's clock does.
fn writer_first() -> Vec<Option<UnitId>> {
    let mut schedule = vec![Some(SUBMITTER)];
    schedule.extend(std::iter::repeat_n(Some(WRITER), WRITER_STEPS as usize));
    schedule
}

#[test]
fn a_store_run_ahead_of_the_lagging_counter_changes_committed_memory() {
    let (default_hash, default_bytes) = run_with(Vec::new());
    let (moved_hash, moved_bytes) = run_with(writer_first());
    assert_ne!(default_hash, moved_hash);
    assert_eq!(
        default_bytes, [WRITER_STEPS as u8; 4],
        "by default the transfer lands before unit 1's last store",
    );
    assert_eq!(
        moved_bytes, TRANSFERRED,
        "the counter's clock holds the landing back past a store run ahead of it",
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
    // `a_store_run_ahead_of_the_lagging_counter_changes_committed_memory`
    // writes by hand.
    let (diverging, _) = run_with(writer_first());
    assert_ne!(result.baseline_hash, diverging);
    assert!(
        result
            .schedules
            .iter()
            .any(|record| !record.truncated && record.memory_hash == diverging),
        "the search reached the schedule that lands the transfer last",
    );
}
