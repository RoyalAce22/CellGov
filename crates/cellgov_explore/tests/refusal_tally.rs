//! `schedules_refused` against the `schedules_truncated` it sits inside.
//!
//! Both tallies answer for alternates. A refusal is one way an alternate
//! truncates, so the refused count is a subset of the truncated one, and
//! neither can exceed the number of alternates the search recorded.
//!
//! A baseline counted into one tally and not the other breaks that
//! without breaking any exit code, because `report.rs` prints the two
//! numbers and nothing recomputes them.
//!
//! Two of the workloads here stop short by faulting and one by a
//! refused commit. A guest fault is not a refusal, so a faulting
//! alternate moves the truncated count alone; only the model declining
//! a step or its commit raises the refused one. Both are here so the
//! zero a fault leaves is a measurement rather than a count nothing in
//! the file could move.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::backtrack::explore_backtrack;
use cellgov_explore::classify::ExplorationResult;
use cellgov_explore::config::ExplorationConfig;
use cellgov_explore::explorer::explore_window;
use cellgov_explore::StopClass;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{CountingUnit, WritingUnit};
use cellgov_time::Budget;

/// The byte the order of two units decides the value of. Guest memory
/// starts zeroed, so a unit reads zero here unless the writer went
/// first.
const GATE: u64 = 32;

/// A baseline that faults on its first step, with no alternate to run.
///
/// One unit, so no schedule branches. The baseline stops short and the
/// search records no alternate at all, which is the shape that put a
/// count beside zero truncations.
fn baseline_faults_alone() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::FaultIfZero { addr: GATE }, FakeOp::End])
    });
    rt
}

/// A baseline that finishes and an alternate that faults.
///
/// The writer holds the lower id, so the baseline sets the gate before
/// the faulter reads it and runs itself out. Reversing the pair is the
/// alternate, and it faults. A stop short reaches the tally only from
/// an alternate: a baseline that stops short records none, because a
/// truncated execution's races are never read.
fn an_alternate_faults() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    let gate = ByteRange::new(GuestAddr::new(GATE), 1).unwrap();
    rt.register_unit_with(|id| WritingUnit::of_value(id, 1, gate, 0xaa));
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::FaultIfZero { addr: GATE }, FakeOp::End])
    });
    rt
}

/// Bytes of guest memory. The indexed store below strides by it, so
/// any index past zero addresses memory the commit pipeline refuses.
const MEMORY: usize = 64;

/// A baseline that finishes and an alternate the commit pipeline
/// refuses.
///
/// The indexed store takes its address from the gate byte, so the order
/// of the two units decides whether that address is inside guest
/// memory. The reader holds the lower id, so the baseline reads a zero
/// gate and stores at `base`. The reversal reads `0xaa` first, indexes
/// past the end of memory, and the commit refuses the write as out of
/// range -- which is a refusal and not a fault.
fn an_alternate_is_refused() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(MEMORY), Budget::new(4), 200);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::SharedLoad { addr: GATE, len: 1 },
                FakeOp::SharedStoreIndexed {
                    base: 0,
                    stride: MEMORY as u64,
                    len: 4,
                },
                FakeOp::End,
            ],
        )
    });
    let gate = ByteRange::new(GuestAddr::new(GATE), 1).unwrap();
    rt.register_unit_with(|id| WritingUnit::of_value(id, 1, gate, 0xaa));
    rt
}

/// No refusal anywhere, as the control.
fn nothing_refuses() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    rt.register_unit_with(|id| CountingUnit::new(id, 2));
    rt.register_unit_with(|id| CountingUnit::new(id, 2));
    rt
}

/// What the two tallies promise each other, and what each promises the
/// records it summarizes.
fn hold_the_tally(name: &str, result: &ExplorationResult) {
    assert!(
        result.schedules_refused <= result.schedules_truncated,
        "{name}: a refusal is one way an alternate truncates, so the \
         refused count sits inside the truncated one: refused={} truncated={}",
        result.schedules_refused,
        result.schedules_truncated,
    );
    assert!(
        result.schedules_truncated <= result.schedules.len(),
        "{name}: an alternate the search never recorded cannot have \
         truncated: truncated={} records={}",
        result.schedules_truncated,
        result.schedules.len(),
    );
    let recorded = result.schedules.iter().filter(|r| r.truncated).count();
    assert_eq!(
        result.schedules_truncated, recorded,
        "{name}: the tally and the records disagree",
    );
    // Derived from the records rather than compared against a constant:
    // the first assertion above holds for a count stuck at zero, and
    // this one does not.
    let refused = result
        .schedules
        .iter()
        .filter(|r| r.stop.class() == StopClass::Refusal)
        .count();
    assert_eq!(
        result.schedules_refused, refused,
        "{name}: the tally counts a stop the records do not call a refusal",
    );
}

#[test]
fn both_searches_hold_the_tally_over_every_workload() {
    let config = ExplorationConfig::default();
    for (name, build) in [
        ("faults alone", baseline_faults_alone as fn() -> Runtime),
        ("an alternate faults", an_alternate_faults),
        ("an alternate is refused", an_alternate_is_refused),
        ("nothing refuses", nothing_refuses),
    ] {
        hold_the_tally(&format!("optimal/{name}"), &explore_window(build, &config));
        hold_the_tally(
            &format!("scaffold/{name}"),
            &explore_backtrack(build, &config),
        );
    }
}

/// The case the tally used to get wrong, stated on its own.
///
/// The baseline stops short and there is no alternate, so both tallies
/// are zero: they answer for alternates, and there were none.
#[test]
fn a_faulting_baseline_with_no_alternate_counts_neither() {
    let config = ExplorationConfig::default();
    for (name, result) in [
        ("optimal", explore_window(baseline_faults_alone, &config)),
        (
            "scaffold",
            explore_backtrack(baseline_faults_alone, &config),
        ),
    ] {
        assert!(
            result.baseline_stop.is_truncated(),
            "{name}: the premise is a baseline that stopped short",
        );
        assert!(result.schedules.is_empty(), "{name}");
        assert_eq!(result.schedules_refused, 0, "{name}");
        assert_eq!(result.schedules_truncated, 0, "{name}");
        assert!(
            result.bounds_hit,
            "{name}: the baseline still bounds the search",
        );
    }
}

/// An alternate that stops short moves the truncated tally, which is
/// what keeps the cases above from passing on a tally stuck at zero.
///
/// It moves only that one. The guest's own step failed, and the model
/// refused nothing, so the two counts part company here.
#[test]
fn a_faulting_alternate_truncates_without_refusing() {
    let result = explore_window(an_alternate_faults, &ExplorationConfig::default());
    assert!(
        !result.baseline_stop.is_truncated(),
        "the premise is a baseline that finished: {}",
        result.baseline_stop,
    );
    assert!(
        !result.schedules.is_empty(),
        "the reversal is the alternate this case needs",
    );
    assert_eq!(
        result.schedules_truncated, 1,
        "the reversed order faults, so the alternate stops short",
    );
    assert_eq!(
        result.schedules_refused, 0,
        "and a guest fault is not the model declining a step",
    );
}

/// A refused alternate raises the refused count, which is the path the
/// tally exists to watch.
///
/// The cases above all leave that count at zero, so without this one a
/// search that never counted a refusal at all would satisfy the file.
#[test]
fn a_refused_alternate_raises_the_refused_count() {
    let result = explore_window(an_alternate_is_refused, &ExplorationConfig::default());
    assert!(
        !result.baseline_stop.is_truncated(),
        "the premise is a baseline that finished: {}",
        result.baseline_stop,
    );
    assert!(
        !result.schedules.is_empty(),
        "the reversal is the alternate this case needs",
    );
    let refused: Vec<&cellgov_explore::ScheduleRecord> = result
        .schedules
        .iter()
        .filter(|r| r.stop.class() == StopClass::Refusal)
        .collect();
    assert!(
        !refused.is_empty(),
        "the reversed order indexes past the end of memory, so its commit is refused: {:?}",
        result
            .schedules
            .iter()
            .map(|r| r.stop.to_string())
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        result.schedules_refused,
        refused.len(),
        "every refused record reaches the tally",
    );
    assert!(
        result.schedules_refused <= result.schedules_truncated,
        "a refusal is one way an alternate truncates: refused={} truncated={}",
        result.schedules_refused,
        result.schedules_truncated,
    );
}

/// The two searches report the same tally for the same workload.
#[test]
fn the_two_searches_agree_on_the_tally() {
    let config = ExplorationConfig::default();
    for (name, build) in [
        ("faults alone", baseline_faults_alone as fn() -> Runtime),
        ("nothing refuses", nothing_refuses),
    ] {
        let optimal = explore_window(build, &config);
        let scaffold = explore_backtrack(build, &config);
        assert_eq!(
            (optimal.schedules_refused, optimal.schedules_truncated),
            (scaffold.schedules_refused, scaffold.schedules_truncated),
            "{name}: the searches disagree on what the workload refused",
        );
    }
}
