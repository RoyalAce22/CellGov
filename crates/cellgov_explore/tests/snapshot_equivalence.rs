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
    let log = observe_decisions(&mut rt_baseline);
    let baseline_hash = rt_baseline.memory().content_hash();

    let total_branching_points = log.branching_count();
    if total_branching_points == 0 {
        return None;
    }

    let mut schedules = Vec::new();
    let mut bounds_hit = false;
    let mut found_divergence = false;
    let mut schedules_pruned = 0usize;

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
            run_to_stall(&mut rt, config.max_steps_per_run);
            let hash = rt.memory().content_hash();
            if hash != baseline_hash {
                found_divergence = true;
            }
            schedules.push(ScheduleRecord {
                branch_step: bp.step,
                alternate_choice: alt,
                memory_hash: hash,
            });
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
    })
}

/// Whole-struct equality (catches forgotten fields) plus
/// field-level fallback for sharper failure messages.
fn assert_equivalent<F>(make_runtime: F, config: &ExplorationConfig, scenario: &str)
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
                return;
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
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
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
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
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
            rt.registry_mut().register_with(|id| {
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
            rt.registry_mut().register_with(|id| {
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
        rt.registry_mut().register_with(|id| {
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
        rt.registry_mut().register_with(|id| {
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
    assert_equivalent(
        make_disjoint_writes(),
        &ExplorationConfig::default(),
        "disjoint_writes",
    );
}

#[test]
fn equivalence_overlapping_writes() {
    assert_equivalent(
        make_overlapping_writes(),
        &ExplorationConfig::default(),
        "overlapping_writes",
    );
}

#[test]
fn equivalence_three_overlapping_writes() {
    assert_equivalent(
        make_three_overlapping_writes(),
        &ExplorationConfig::default(),
        "three_overlapping_writes",
    );
}

#[test]
fn equivalence_atomic_contention() {
    assert_equivalent(
        make_atomic_contention(),
        &ExplorationConfig::default(),
        "atomic_contention",
    );
}

#[test]
fn equivalence_dma_overlapping_writes() {
    assert_equivalent(
        make_dma_overlapping_writes(),
        &ExplorationConfig::default(),
        "dma_overlapping_writes",
    );
}

/// `max_schedules = 1` against 2 alternates exercises the
/// `'outer: break` path that default-config scenarios don't touch.
#[test]
fn equivalence_three_overlapping_with_tight_bounds() {
    // Spread is intentional: a future ExplorationConfig field
    // should silently inherit the default rather than break the
    // test on a literal-init compile error.
    #[allow(clippy::needless_update)]
    let config = ExplorationConfig {
        max_schedules: 1,
        max_steps_per_run: 100,
        ..ExplorationConfig::default()
    };
    assert_equivalent(
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
    assert_equivalent(
        make_three_overlapping_writes(),
        &config,
        "three_overlapping_exact_bounds",
    );
}
