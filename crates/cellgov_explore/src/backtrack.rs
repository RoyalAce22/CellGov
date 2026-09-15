//! Backtrack-set dynamic partial-order reduction
//! [FlanaganGodefroid2005 p:4 s:3].
//!
//! The search runs one execution, walks its races, and adds a
//! backtrack point at the earlier event of each race; every backtrack
//! point becomes one schedule to replay, and two points can reach one
//! equivalence class. Figure 3 reads the next transition of every
//! process at every state, including a disabled one
//! [FlanaganGodefroid2005 p:5 s:3]; this search learns a step's
//! footprint only once the step runs, so a unit blocked through every
//! execution names no backtrack point.

use crate::classify::{BaselineRun, ExplorationResult, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::decision::DecisionLog;
use crate::execution::{Execution, Race};
use crate::observer::observe_decisions_bounded;
use crate::prescribed::PrescribedScheduler;
use crate::util::{classify_iteration, AlternateIteration, StopClass};
use cellgov_event::UnitId;
use std::collections::BTreeSet;

/// One schedule to replay: the units to force, step by step, from step
/// zero.
///
/// The last entry is the backtrack point's choice; the round-robin
/// fallback chooses every step past it, so one prefix stands for one
/// subtree.
type Prefix = Vec<UnitId>;

/// A backtrack point: the schedule that reaches it, and the step it
/// forces a different unit at.
struct Candidate {
    prefix: Prefix,
    /// Step the last entry of `prefix` forces.
    branch_step: usize,
}

/// Run backtrack-set DPOR on a workload.
///
/// The search calls `make_runtime` once per execution, so it must build
/// the same workload every time.
///
/// This search can run more than one execution per class, so it claims
/// no [`ExplorationResult::classes_explored`]; `push_candidates` says
/// what [`ExplorationResult::schedules_pruned`] and
/// [`ExplorationResult::reversals_dropped`] count here.
pub fn explore_backtrack<F>(mut make_runtime: F, config: &ExplorationConfig) -> ExplorationResult
where
    F: FnMut() -> cellgov_core::Runtime,
{
    let mut rt = make_runtime();
    let (log, baseline_stop) = observe_decisions_bounded(&mut rt, config.max_steps_per_run);
    let baseline = BaselineRun {
        hash: rt.observable_hash(),
        steps: log.len(),
        stop: baseline_stop,
    };
    let total_branching_points = log.branching_count();
    // Each replay builds and drops its own runtime, so a break that
    // only one replay found is readable in that iteration alone.
    let mut first_invariant_break = rt.lv2_host().observability().first_invariant_break_line();

    // Every candidate ends with the unit it forces, so the baseline's
    // empty prefix is not one of them and needs no entry here.
    let mut seen: BTreeSet<Prefix> = BTreeSet::new();
    let mut worklist: Vec<Candidate> = Vec::new();
    let mut schedules_pruned = 0usize;
    let mut dropped_reversals = 0usize;
    // One entry per undeliverable reversal: its prefix and the race's
    // two events. Two races over the same pair of positions can still
    // name different units, so the key carries the events whole.
    let mut dropped_seen: BTreeSet<(Prefix, Race)> = BTreeSet::new();

    // A truncated execution's race set covers a prefix of the
    // workload, so it names no backtrack point worth replaying.
    if !baseline_stop.is_truncated() {
        push_candidates(
            &log,
            &mut seen,
            &mut dropped_seen,
            &mut worklist,
            &mut schedules_pruned,
            &mut dropped_reversals,
        );
    }

    let mut iter = AlternateIteration {
        schedules: Vec::new(),
        bounds_hit: false,
        found_divergence: false,
        schedules_pruned,
        schedules_truncated: 0,
        schedules_refused: 0,
    };

    while let Some(candidate) = worklist.pop() {
        if iter.schedules.len() >= config.max_schedules {
            iter.bounds_hit = true;
            break;
        }
        let mut rt = make_runtime();
        rt.set_scheduler(PrescribedScheduler::new(
            candidate.prefix.iter().copied().map(Some).collect(),
        ));
        let (log, stop) = observe_decisions_bounded(&mut rt, config.max_steps_per_run);
        let hash = rt.observable_hash();
        if first_invariant_break.is_none() {
            first_invariant_break = rt.lv2_host().observability().first_invariant_break_line();
        }
        let truncated = stop.is_truncated();
        // The fallback can drift a replay off its prefix, and the
        // record would then name a `branch_step` it never reached. A
        // truncated replay is short of its prefix because a truncating
        // stop gets no point, which is no drift.
        debug_assert!(
            (truncated || log.points().len() >= candidate.prefix.len())
                && log
                    .points()
                    .iter()
                    .zip(&candidate.prefix)
                    .all(|(point, forced)| point.chosen == *forced),
            "the replay did not reproduce its prescribed prefix",
        );
        if truncated {
            iter.schedules_truncated += 1;
            iter.bounds_hit = true;
            if stop.class() == StopClass::Refusal {
                iter.schedules_refused += 1;
            }
        } else {
            if hash != baseline.hash {
                iter.found_divergence = true;
            }
            push_candidates(
                &log,
                &mut seen,
                &mut dropped_seen,
                &mut worklist,
                &mut iter.schedules_pruned,
                &mut dropped_reversals,
            );
        }
        iter.schedules.push(ScheduleRecord {
            branch_step: candidate.branch_step,
            alternate_choice: *candidate
                .prefix
                .last()
                .expect("a candidate forces at least one step"),
            memory_hash: hash,
            stop,
            truncated,
        });
    }

    if baseline.stop.is_truncated() {
        iter.mark_baseline_truncated();
    }

    let mut result = classify_iteration(
        iter,
        baseline,
        total_branching_points,
        first_invariant_break,
    );
    result.reversals_dropped = dropped_reversals;
    result
}

/// Add a backtrack point for every race in `log`'s execution.
///
/// The point sits at the earlier event of the race and forces the
/// later event's unit. When that unit was not runnable at the point,
/// the search forces every other runnable unit instead
/// [FlanaganGodefroid2005 p:5 s:Figure 3], as the paper's own
/// implementation does [FlanaganGodefroid2005 p:6 s:4.1].
///
/// `dropped` counts the races that fallback leaves no unit to force
/// for; `suppressed` counts candidates an earlier one already named,
/// which cost no class.
fn push_candidates(
    log: &DecisionLog,
    seen: &mut BTreeSet<Prefix>,
    dropped_seen: &mut BTreeSet<(Prefix, Race)>,
    worklist: &mut Vec<Candidate>,
    suppressed: &mut usize,
    dropped: &mut usize,
) {
    let execution = Execution::from_log(log);
    let relation = execution.happens_before();
    let points = log.points();
    for race in execution.races(&relation) {
        let at = race.first.index;
        let point = points
            .get(at)
            .expect("the execution and its races come from these points");
        let forced: Vec<UnitId> = if point.runnable.contains(&race.second.unit) {
            vec![race.second.unit]
        } else {
            point
                .runnable
                .iter()
                .copied()
                .filter(|unit| *unit != point.chosen)
                .collect()
        };
        // Every replay sharing this prefix re-reads the same race, so
        // `dropped_seen` counts it once.
        if forced.is_empty() {
            let prefix: Prefix = points[..at].iter().map(|p| p.chosen).collect();
            if dropped_seen.insert((prefix, race)) {
                *dropped += 1;
            }
        }
        for unit in forced {
            let mut prefix: Prefix = points[..at].iter().map(|p| p.chosen).collect();
            prefix.push(unit);
            if seen.insert(prefix.clone()) {
                worklist.push(Candidate {
                    prefix,
                    branch_step: at,
                });
            } else {
                *suppressed += 1;
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/backtrack_tests.rs"]
mod tests;
