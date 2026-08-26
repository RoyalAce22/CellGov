//! Liveness gate for the `effects_buf`-empty debug assert in
//! `Runtime::snapshot()`, which holds only at a batch boundary. The
//! per-step loop never snapshots; only the schedule-exploration
//! observer (`cellgov_explore::observe_decisions_with_snapshots`)
//! does, once per branching point, so the assert is evaluated
//! exactly `ExplorationResult::total_branching_points` times. Each
//! foundation scenario must report that count > 0 or the assert's
//! silence is vacuous; the declared lower bounds are diagnostic only.

#![allow(
    clippy::print_stderr,
    reason = "integration test: stderr carries diagnostics"
)]

use cellgov_explore::{explore, ExplorationConfig};
use cellgov_testkit::fixtures;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotStatus {
    /// The scenario produces at least one branching point during
    /// baseline run; `Runtime::snapshot` is called there, and the
    /// effects_buf-empty debug_assert is evaluated. Witness count
    /// (`total_branching_points`) must be > 0.
    /// `expected_at_least` is the empirical lower bound, surfaced
    /// in the diagnostic but not asserted == exactly.
    Reached { expected_at_least: usize },
}

struct Case {
    name: &'static str,
    factory: fn() -> cellgov_testkit::fixtures::ScenarioFixture,
    status: SnapshotStatus,
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
        status: SnapshotStatus::Reached {
            expected_at_least: 5,
        },
        reason: "write_conflict_scenario(3) produces 5 branching points where 3 units compete for the same memory (measured 2026-06-04)",
    },
    Case {
        name: "fairness",
        factory: fairness_factory,
        status: SnapshotStatus::Reached {
            expected_at_least: 14,
        },
        reason: "round_robin_fairness_scenario(3, 5) produces 14 branching points across 3 units, 5 steps each (measured 2026-06-04)",
    },
    Case {
        name: "mailbox",
        factory: mailbox_factory,
        status: SnapshotStatus::Reached {
            expected_at_least: 1,
        },
        reason: "mailbox_roundtrip_scenario(0x42) produces 1 branching point at the mailbox handshake (measured 2026-06-04)",
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

    match case.status {
        SnapshotStatus::Reached { expected_at_least } => {
            assert!(
                result.total_branching_points > 0,
                "snapshot_effects_tripwire {}: declared Reached \
                 (expected_at_least={expected_at_least}, {}) but \
                 total_branching_points=0. The observer never reached a >=2 runnable \
                 set, so Runtime::snapshot was never called and the effects_buf-empty \
                 debug_assert's silence is vacuous on this scenario. Either trajectory \
                 regressed or this entry should be re-evaluated.",
                case.name,
                case.reason
            );
            if result.total_branching_points < expected_at_least {
                eprintln!(
                    "snapshot_effects_tripwire {}: observed total_branching_points={} \
                     below documented expected_at_least={expected_at_least} ({}). The \
                     lower-bound assertion still passes (count > 0); update the \
                     declaration to the new baseline if the shift is intentional.",
                    case.name, result.total_branching_points, case.reason
                );
            }
        }
    }
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

#[test]
fn declared_cases_cover_every_status_variant() {
    // Pins the no-vacuous-variant intent: today every CASES entry
    // uses Reached. If a future variant (NoBranching) is added,
    // the run_case match arm must enforce its semantics. The
    // exhaustive compile-time match already covers that; this
    // test pins it at runtime so a reviewer sees the gap before
    // a vacuous variant ships.
    for case in CASES {
        match case.status {
            SnapshotStatus::Reached { expected_at_least } => {
                assert!(
                    expected_at_least > 0,
                    "snapshot_effects_tripwire {}: declared Reached(expected_at_least=0) \
                     is structurally allowed but vacuous (silence equals zero by default). \
                     If the scenario legitimately has no branching points, audit whether \
                     the gate is still meaningful for it.",
                    case.name
                );
            }
        }
    }
}
