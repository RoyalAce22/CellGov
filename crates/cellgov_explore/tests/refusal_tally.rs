//! `schedules_refused` against the `schedules_truncated` it sits inside.
//!
//! Both tallies answer for alternates. A refusal is one way an alternate
//! truncates, so the refused count is a subset of the truncated one, and
//! neither can exceed the number of alternates the search recorded.
//!
//! A baseline counted into one tally and not the other breaks that
//! without breaking any exit code, because `report.rs` prints the two
//! numbers and nothing recomputes them.

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
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{CountingUnit, WritingUnit};
use cellgov_time::Budget;

/// The byte the fault turns on. Guest memory starts zeroed, so a unit
/// reading it faults unless something wrote it first.
const GATE: u64 = 32;

/// A baseline that faults on its first step, with no alternate to run.
///
/// One unit, so no schedule branches. The baseline truncates on a
/// refusal and the search records no alternate at all, which is the
/// shape that put a refusal beside zero truncations.
fn baseline_refuses_alone() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::FaultIfZero { addr: GATE }, FakeOp::End])
    });
    rt
}

/// A baseline that finishes and an alternate that refuses.
///
/// The writer holds the lower id, so the baseline sets the gate before
/// the faulter reads it and runs itself out. Reversing the pair is the
/// alternate, and it faults. That is the only shape where a refusal
/// reaches the tally at all: a refusing baseline records no alternate,
/// because a truncated execution's races are never read.
fn an_alternate_refuses() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    let gate = ByteRange::new(GuestAddr::new(GATE), 1).unwrap();
    rt.register_unit_with(|id| WritingUnit::of_value(id, 1, gate, 0xaa));
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::FaultIfZero { addr: GATE }, FakeOp::End])
    });
    rt
}

/// No refusal anywhere, as the control.
fn nothing_refuses() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 200);
    rt.register_unit_with(|id| CountingUnit::new(id, 2));
    rt.register_unit_with(|id| CountingUnit::new(id, 2));
    rt
}

/// The three things the two tallies promise each other.
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
}

#[test]
fn both_searches_hold_the_tally_over_every_workload() {
    let config = ExplorationConfig::default();
    for (name, build) in [
        ("refuses alone", baseline_refuses_alone as fn() -> Runtime),
        ("an alternate refuses", an_alternate_refuses),
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
/// The baseline refuses and there is no alternate, so both tallies are
/// zero: they answer for alternates, and there were none.
#[test]
fn a_refusing_baseline_with_no_alternate_counts_neither() {
    let config = ExplorationConfig::default();
    for (name, result) in [
        ("optimal", explore_window(baseline_refuses_alone, &config)),
        (
            "scaffold",
            explore_backtrack(baseline_refuses_alone, &config),
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

/// A refusing alternate is counted, which is the path the tally reaches
/// through.
///
/// Without this the file would only show zeroes, and a tally that
/// counted nothing at all would satisfy every other case here.
#[test]
fn a_refusing_alternate_is_counted_in_both_tallies() {
    let result = explore_window(an_alternate_refuses, &ExplorationConfig::default());
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
        result.schedules_refused, 1,
        "the alternate faulted, and a fault is a refusal",
    );
    assert_eq!(
        result.schedules_truncated, 1,
        "and the refusal is one of the ways it truncated",
    );
}

/// The two searches report the same tally for the same workload.
#[test]
fn the_two_searches_agree_on_the_tally() {
    let config = ExplorationConfig::default();
    for (name, build) in [
        ("refuses alone", baseline_refuses_alone as fn() -> Runtime),
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
