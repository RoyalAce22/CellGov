//! Pins that the snapshot-restore [`cellgov_explore::explore`] path
//! produces a byte-identical [`ExplorationResult`] to a local
//! factory-replay reference. Classification outcomes alone can mask
//! schedule-shifting field divergences that happen to land on the
//! same memory hash; whole-struct `PartialEq` catches them.
//!
//! The factory reference reuses the production `observe_decisions`,
//! `PrescribedScheduler`, and `cellgov_explore::util::*` helpers, so
//! this test isolates the snapshot/restore axis -- it does not
//! re-validate those helpers.
//!
//! Coverage gap: scenarios all use a fixed `Budget` and `max_steps`;
//! construction-param variation is pinned by the `captured_*` debug
//! assertions in `cellgov_core::runtime::snapshot`, not here.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: .unwrap() panics on unexpected failure are the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{
    explore, observe_decisions,
    util::{build_overrides, run_to_stall},
    ExplorationConfig, ExplorationResult, OutcomeClass, PrescribedScheduler, ScheduleRecord,
};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// Factory-replay reference: rebuilds the runtime from
/// `make_runtime` per alternate. Mirrors
/// [`cellgov_explore::util::for_each_alternate`]'s `'outer: for`
/// shape; `bounds_hit` semantics follow that function's definition.
fn explore_via_factory_replay<F>(
    mut make_runtime: F,
    config: &ExplorationConfig,
) -> Option<ExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let mut rt_baseline = make_runtime();
    let (log, baseline_stop) = observe_decisions(&mut rt_baseline);
    let baseline_hash = rt_baseline.memory().content_hash();

    let total_branching_points = log.branching_count();
    if total_branching_points == 0 {
        return None;
    }

    let mut schedules = Vec::new();
    let mut bounds_hit = false;
    let mut found_divergence = false;
    let mut schedules_pruned = 0usize;
    let mut schedules_truncated = 0usize;

    'outer: for bp in log.branching_points() {
        let default_choice = bp.chosen;
        for &alt in &bp.runnable {
            if alt == default_choice {
                continue;
            }
            if schedules.len() >= config.max_schedules {
                bounds_hit = true;
                break 'outer;
            }
            if let Some(alt_agg) = log.aggregate_footprint(alt) {
                if let Some(def_agg) = log.aggregate_footprint(default_choice) {
                    if !def_agg.conflicts(&alt_agg) {
                        schedules_pruned += 1;
                        continue;
                    }
                }
            }

            let overrides = build_overrides(bp.step, alt);
            let mut rt = make_runtime();
            rt.set_scheduler(PrescribedScheduler::new(overrides));
            let stop = run_to_stall(&mut rt, config.max_steps_per_run);
            let hash = rt.memory().content_hash();
            let truncated = stop.is_truncated();
            if truncated {
                schedules_truncated += 1;
                bounds_hit = true;
            } else if hash != baseline_hash {
                found_divergence = true;
            }
            schedules.push(ScheduleRecord {
                branch_step: bp.step,
                alternate_choice: alt,
                memory_hash: hash,
                truncated,
            });
        }
    }

    // A prefix baseline withdraws every divergence claim, mirroring
    // `AlternateIteration::mark_baseline_truncated`.
    // A prefix baseline withdraws every divergence claim, mirroring
    // `AlternateIteration::mark_baseline_truncated`.
    if baseline_stop.is_truncated() {
        found_divergence = false;
        bounds_hit = true;
        schedules_truncated = schedules.len();
        for record in &mut schedules {
            record.truncated = true;
        }
    }

    let outcome = if found_divergence {
        OutcomeClass::ScheduleSensitive
    } else if bounds_hit {
        OutcomeClass::Inconclusive
    } else {
        OutcomeClass::ScheduleStable
    };
    Some(ExplorationResult {
        baseline_hash,
        schedules,
        outcome,
        total_branching_points,
        bounds_hit,
        schedules_pruned,
        schedules_truncated,
    })
}

/// Whole-struct equality (catches forgotten fields) plus
/// field-level fallback for sharper failure messages.
fn assert_equivalent<F>(
    make_runtime: F,
    config: &ExplorationConfig,
    scenario: &str,
) -> ExplorationResult
where
    F: FnMut() -> Runtime + Clone,
{
    let mut a = make_runtime.clone();
    let mut b = make_runtime;
    let snap_path = explore(&mut a, config);
    let factory_path = explore_via_factory_replay(&mut b, config);
    match (snap_path, factory_path) {
        (Some(s), Some(f)) => {
            if s == f {
                return s;
            }
            // Per-field dump on mismatch so the first divergent
            // axis names itself rather than requiring a Debug-blob
            // diff. Whole-struct equality above is the canary.
            assert_eq!(
                s.baseline_hash, f.baseline_hash,
                "{scenario}: baseline_hash differs"
            );
            assert_eq!(s.outcome, f.outcome, "{scenario}: outcome differs");
            assert_eq!(
                s.total_branching_points, f.total_branching_points,
                "{scenario}: total_branching_points differs"
            );
            assert_eq!(s.bounds_hit, f.bounds_hit, "{scenario}: bounds_hit differs");
            assert_eq!(
                s.schedules_pruned, f.schedules_pruned,
                "{scenario}: schedules_pruned differs"
            );
            assert_eq!(
                s.schedules_truncated, f.schedules_truncated,
                "{scenario}: schedules_truncated differs"
            );
            assert_eq!(
                s.schedules.len(),
                f.schedules.len(),
                "{scenario}: schedules count differs"
            );
            for (i, (sr, fr)) in s.schedules.iter().zip(f.schedules.iter()).enumerate() {
                assert_eq!(sr, fr, "{scenario}: schedule[{i}] differs");
            }
            // If we get here, the whole-struct check failed but no
            // per-field check did. That means a new field was added
            // to ExplorationResult without a paired check below.
            panic!(
                "{scenario}: ExplorationResult differs but no per-field assertion fired -- \
                 a new field was added to ExplorationResult without updating this test"
            );
        }
        (None, None) => {
            panic!(
                "{scenario}: both paths returned None -- scenario has no \
                 branching points and exercises nothing useful; the test setup \
                 needs >= 2 runnable units"
            );
        }
        (Some(_), None) | (None, Some(_)) => {
            panic!("{scenario}: snapshot and factory paths disagree on Some/None")
        }
    }
}

fn make_disjoint_writes() -> impl FnMut() -> Runtime + Clone {
    || {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xBB),
                    FakeOp::SharedStore { addr: 8, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt
    }
}

fn make_overlapping_writes() -> impl FnMut() -> Runtime + Clone {
    || {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xBB),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt
    }
}

fn make_three_overlapping_writes() -> impl FnMut() -> Runtime + Clone {
    || {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        for v in [0xAAu32, 0xBB, 0xCC] {
            rt.register_unit_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(v),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
        }
        rt
    }
}

fn make_atomic_contention() -> impl FnMut() -> Runtime + Clone {
    || {
        let mem = GuestMemory::new(256);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        for v in [0xAAu32, 0xBB] {
            rt.register_unit_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(v),
                        FakeOp::ReservationAcquire { line_addr: 0x80 },
                        FakeOp::ConditionalStore { addr: 0x80, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
        }
        rt
    }
}

/// `DmaPut` to `0x80..0x90` and a direct store to `0x88..0x8C`
/// overlap, so pruning cannot skip the alternate and the
/// `DmaQueue` clone path through `RuntimeSnapshot::dma_queue`
/// gets exercised.
fn make_dma_overlapping_writes() -> impl FnMut() -> Runtime + Clone {
    || {
        let mem = GuestMemory::new(256);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::DmaPut {
                        src: 0x40,
                        dst: 0x80,
                        len: 16,
                    },
                    FakeOp::End,
                ],
            )
        });
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xCC),
                    FakeOp::SharedStore { addr: 0x88, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt
    }
}

#[test]
fn equivalence_disjoint_writes() {
    let _ = assert_equivalent(
        make_disjoint_writes(),
        &ExplorationConfig::default(),
        "disjoint_writes",
    );
}

#[test]
fn equivalence_overlapping_writes() {
    let _ = assert_equivalent(
        make_overlapping_writes(),
        &ExplorationConfig::default(),
        "overlapping_writes",
    );
}

#[test]
fn equivalence_three_overlapping_writes() {
    let _ = assert_equivalent(
        make_three_overlapping_writes(),
        &ExplorationConfig::default(),
        "three_overlapping_writes",
    );
}

#[test]
fn equivalence_atomic_contention() {
    let _ = assert_equivalent(
        make_atomic_contention(),
        &ExplorationConfig::default(),
        "atomic_contention",
    );
}

#[test]
fn equivalence_dma_overlapping_writes() {
    let _ = assert_equivalent(
        make_dma_overlapping_writes(),
        &ExplorationConfig::default(),
        "dma_overlapping_writes",
    );
}

/// `max_schedules = 1` against 2 alternates exercises the
/// `'outer: break` path that default-config scenarios don't touch.
#[test]
fn equivalence_three_overlapping_with_tight_bounds() {
    // Spread keeps a future ExplorationConfig field inheriting the
    // default rather than breaking the test on a literal-init compile
    // error.
    #[allow(clippy::needless_update)]
    let config = ExplorationConfig {
        max_schedules: 1,
        max_steps_per_run: 100,
        ..ExplorationConfig::default()
    };
    let _ = assert_equivalent(
        make_three_overlapping_writes(),
        &config,
        "three_overlapping_tight_bounds",
    );
}

/// `max_schedules` equal to alternate count: pins that the two
/// paths agree on `bounds_hit` semantics in this corner.
#[test]
fn equivalence_three_overlapping_with_exact_bounds() {
    #[allow(clippy::needless_update)]
    let config = ExplorationConfig {
        max_schedules: 2,
        max_steps_per_run: 100,
        ..ExplorationConfig::default()
    };
    let _ = assert_equivalent(
        make_three_overlapping_writes(),
        &config,
        "three_overlapping_exact_bounds",
    );
}

/// Both units need three steps but the runtime's own cap refuses the
/// fifth, so the baseline stops on `StepError` with work still
/// runnable. Every recorded hash is then a prefix hash.
fn make_baseline_truncated_by_step_cap() -> impl FnMut() -> Runtime + Clone {
    || {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 4);
        for v in [0xAAu32, 0xBB] {
            rt.register_unit_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(v),
                        FakeOp::SharedStore { addr: 0, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
        }
        rt
    }
}

/// The truncation rule is the one part of `for_each_alternate` that no
/// other scenario here reaches: without a scenario that actually stops
/// short, the reference implementation can drop the rule entirely and
/// still match.
#[test]
fn equivalence_holds_when_the_baseline_stops_short() {
    let r = assert_equivalent(
        make_baseline_truncated_by_step_cap(),
        &ExplorationConfig::default(),
        "baseline_truncated_by_step_cap",
    );
    assert_eq!(
        r.outcome,
        OutcomeClass::Inconclusive,
        "a prefix baseline cannot support any verdict but inconclusive",
    );
    assert!(r.bounds_hit);
    assert!(
        !r.schedules.is_empty(),
        "the scenario must explore at least one alternate for the          truncation rule to be under test",
    );
    assert_eq!(
        r.schedules_truncated,
        r.schedules.len(),
        "a prefix baseline taints every record",
    );
    assert!(r.schedules.iter().all(|s| s.truncated));
}

/// Disjoint writers are pruned to nothing, so `bounds_hit` can only
/// come from the baseline rule -- without it a prefix run reports
/// `schedule-stable`.
fn make_pruned_alternates_with_a_truncated_baseline() -> impl FnMut() -> Runtime + Clone {
    || {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 4);
        for (addr, v) in [(0u64, 0xAAu32), (8, 0xBB)] {
            rt.register_unit_with(move |id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(v),
                        FakeOp::SharedStore { addr, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
        }
        rt
    }
}

#[test]
fn a_truncated_baseline_whose_alternates_were_all_pruned_is_not_stable() {
    let r = assert_equivalent(
        make_pruned_alternates_with_a_truncated_baseline(),
        &ExplorationConfig::default(),
        "pruned_alternates_truncated_baseline",
    );
    assert!(
        r.schedules.is_empty() && r.schedules_pruned > 0,
        "the scenario must prune every alternate for the baseline rule to be          the only thing that can set bounds_hit: got {} schedules, {} pruned",
        r.schedules.len(),
        r.schedules_pruned,
    );
    assert_eq!(
        r.outcome,
        OutcomeClass::Inconclusive,
        "a prefix baseline with nothing left to compare is not evidence of stability",
    );
    assert!(r.bounds_hit);
}
