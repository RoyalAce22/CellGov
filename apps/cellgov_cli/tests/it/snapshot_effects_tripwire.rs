//! Liveness gate for the `effects_buf`-empty debug assert in
//! `Runtime::snapshot()`, which holds only at a batch boundary.
//!
//! The per-step loop never snapshots. Schedule exploration does: it
//! snapshots the runtime the caller gives it once, and restores that
//! snapshot for each execution it runs. So every case below evaluates
//! the assert once, and a case that returns a result at all is the
//! witness.

use cellgov_explore::{explore, ExplorationConfig};
use cellgov_testkit::fixtures;

struct Case {
    name: &'static str,
    factory: fn() -> cellgov_testkit::fixtures::ScenarioFixture,
    /// One-line reason a reviewer can challenge.
    reason: &'static str,
}

fn conflict_factory() -> cellgov_testkit::fixtures::ScenarioFixture {
    fixtures::write_conflict_scenario(3)
}

fn fairness_factory() -> cellgov_testkit::fixtures::ScenarioFixture {
    fixtures::round_robin_fairness_scenario(3, 5)
}

fn mailbox_factory() -> cellgov_testkit::fixtures::ScenarioFixture {
    fixtures::mailbox_roundtrip_scenario(0x42)
}

const CASES: &[Case] = &[
    Case {
        name: "conflict",
        factory: conflict_factory,
        reason: "write_conflict_scenario(3) branches where 3 units compete for the same memory",
    },
    Case {
        name: "fairness",
        factory: fairness_factory,
        reason: "round_robin_fairness_scenario(3, 5) branches across 3 units, 5 steps each",
    },
    Case {
        name: "mailbox",
        factory: mailbox_factory,
        reason: "mailbox_roundtrip_scenario(0x42) branches at the mailbox handshake",
    },
];

fn run_case(case: &Case) {
    let config = ExplorationConfig::default();
    let result = explore(|| (case.factory)().build_runtime(), &config).unwrap_or_else(|| {
        panic!(
            "snapshot_effects_tripwire {}: explore returned None ({}). \
             A scenario with no branching points cannot witness the snapshot guard \
             -- either the scenario factory changed to be single-unit or the \
             observer no longer reaches the runnable-len-2 path.",
            case.name, case.reason
        )
    });

    assert!(
        result.total_branching_points > 0,
        "snapshot_effects_tripwire {}: total_branching_points=0 ({}). The observer \
         never reached a >=2 runnable set, so Runtime::snapshot was never called and \
         the effects_buf-empty debug_assert's silence is vacuous on this scenario.",
        case.name,
        case.reason
    );
}

#[test]
fn conflict_scenario_exercises_snapshot_guard() {
    run_case(&CASES[0]);
}

#[test]
fn fairness_scenario_exercises_snapshot_guard() {
    run_case(&CASES[1]);
}

#[test]
fn mailbox_scenario_exercises_snapshot_guard() {
    run_case(&CASES[2]);
}
