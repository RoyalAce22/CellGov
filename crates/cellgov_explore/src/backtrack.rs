//! Backtrack-set dynamic partial-order reduction
//! [FlanaganGodefroid2005 p:4 s:3].
//!
//! The search runs one execution, walks its races, and adds a
//! backtrack point at the earlier event of each race. Every backtrack
//! point becomes one schedule to replay, and each replay yields races
//! of its own. Two backtrack points can reach the same equivalence
//! class, so one class can cost more than one execution.
//!
//! Figure 3 reads the next transition of every process at every state,
//! including a process disabled there [FlanaganGodefroid2005 p:5 s:3].
//! This search learns a step's footprint only once the step runs. A
//! unit that stayed blocked through every execution the search reached
//! names no backtrack point.

use crate::classify::{BaselineRun, ExplorationResult, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::decision::DecisionLog;
use crate::execution::Execution;
use crate::observer::observe_decisions_bounded;
use crate::prescribed::PrescribedScheduler;
use crate::util::{classify_iteration, AlternateIteration, StopClass};
use cellgov_event::UnitId;
use std::collections::BTreeSet;

/// One schedule to replay: the units to force, step by step, from step
/// zero.
///
/// A prefix names a choice for every step up to and including its
/// backtrack point. The round-robin fallback chooses every step past
/// its end, so one prefix stands for one subtree.
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
/// The search calls `make_runtime` once per explored execution, so it
/// must build the same workload every time. Exploration stops at
/// `config.max_schedules` replays and each execution at
/// `config.max_steps_per_run` steps.
///
/// [`classify_iteration`] gives the outcome:
///
/// - only an execution that ran itself out contributes a divergence;
/// - a baseline that stopped short withdraws every claim measured
///   against it.
///
/// [`ExplorationResult::schedules_pruned`] counts candidates dropped
/// because an earlier one already named the same prefix.
pub fn explore_backtrack<F>(mut make_runtime: F, config: &ExplorationConfig) -> ExplorationResult
where
    F: FnMut() -> cellgov_core::Runtime,
{
    let mut rt = make_runtime();
    let (log, baseline_stop) = observe_decisions_bounded(&mut rt, config.max_steps_per_run);
    let baseline = BaselineRun {
        hash: rt.committed_memory_hash(),
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

    // A truncated execution's race set covers a prefix of the
    // workload, so it names no backtrack point worth replaying.
    if !baseline_stop.is_truncated() {
        push_candidates(&log, &mut seen, &mut worklist, &mut schedules_pruned);
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
        let hash = rt.committed_memory_hash();
        if first_invariant_break.is_none() {
            first_invariant_break = rt.lv2_host().observability().first_invariant_break_line();
        }
        // `PrescribedScheduler` falls back to round-robin where a
        // prescribed unit is not runnable. A replay that drifted off
        // its prefix would record a `branch_step` it never reached.
        debug_assert!(
            log.points().len() >= candidate.prefix.len()
                && log
                    .points()
                    .iter()
                    .zip(&candidate.prefix)
                    .all(|(point, forced)| point.chosen == *forced),
            "the replay did not reproduce its prescribed prefix",
        );
        let truncated = stop.is_truncated();
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
            push_candidates(&log, &mut seen, &mut worklist, &mut iter.schedules_pruned);
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

    classify_iteration(
        iter,
        baseline,
        total_branching_points,
        first_invariant_break,
    )
}

/// Add a backtrack point for every race in `log`'s execution.
///
/// The point sits at the earlier event of the race and forces the
/// later event's unit. When that unit was not runnable at the point,
/// the search forces every other runnable unit instead
/// [FlanaganGodefroid2005 p:5 s:Figure 3]. The reversal needs some
/// unit that can reach the later event's state, and the search cannot
/// tell which one does. The paper's own implementation takes the same
/// branch [FlanaganGodefroid2005 p:6 s:4.1].
fn push_candidates(
    log: &DecisionLog,
    seen: &mut BTreeSet<Prefix>,
    worklist: &mut Vec<Candidate>,
    suppressed: &mut usize,
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
