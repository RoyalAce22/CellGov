//! Optimal DPOR: source sets and wakeup trees
//! [Abdulla2017 p:42:24 s:Algorithm 2].
//!
//! The search runs one execution to a maximal sequence, reads its
//! races, and puts the sequence that reverses each race into the wakeup
//! tree at the prefix before the earlier event. The next execution
//! takes the least branch a tree holds. A sleep set carries the units
//! already explored from a prefix; [`crate::wakeup`] says how the tree
//! keeps it from blocking. Each maximal execution stands for one
//! equivalence class.

use crate::classify::{BaselineRun, ExplorationResult, ScheduleRecord};
use crate::config::ExplorationConfig;
use crate::decision::{DecisionLog, DecisionPoint};
use crate::dependency::StepFootprint;
use crate::execution::Execution;
use crate::prescribed::PrescribedScheduler;
use crate::util::{classify_iteration, AlternateIteration, StopClass, StopReason};
use crate::wakeup::{initials, SeqEvent, WakeupTree};
use cellgov_core::{Runtime, RuntimeSnapshot};
use cellgov_event::UnitId;
use std::collections::{BTreeMap, BTreeSet};

/// What one execution of the search produced.
struct Run {
    log: DecisionLog,
    hash: u64,
    invariant_break: Option<String>,
    /// Unit this execution ran at the depth it re-decided at.
    ///
    /// `run_one` reads this before it truncates the frame stack: a
    /// first step the model refuses cuts that frame away, and the
    /// record still names the alternate the run took.
    alternate_choice: Option<UnitId>,
    dropped: Drops,
}

/// Wakeup-tree branches the search gave up, by the site that gave each
/// one up.
///
/// [`ExplorationResult::reversals_dropped`] carries [`Drops::total`];
/// the split is for a test that asks which site answered.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Drops {
    /// [`choose`] met a branch whose head cannot run at the depth
    /// holding it.
    by_choose: usize,
    /// A warp depth met an inherited branch, and the warp woke a
    /// different unit ([`Frame::warp_woke`]).
    at_warp: usize,
    /// Visits at which a warp depth read its own tree.
    ///
    /// `at_warp` moves only on these visits, so a zero there says
    /// nothing until this is above zero.
    at_warp_visits: usize,
}

impl Drops {
    /// Sequences dropped at either site, one per sequence per frame
    /// ([`Frame::dropped`]).
    fn total(self) -> usize {
        self.by_choose + self.at_warp
    }

    fn add(&mut self, other: Self) {
        self.by_choose += other.by_choose;
        self.at_warp += other.at_warp;
        self.at_warp_visits += other.at_warp_visits;
    }
}

/// The search's state at one depth of the execution it runs.
///
/// The prefix this frame belongs to is the units chosen at every
/// shallower depth.
struct Frame {
    /// Unit this execution ran at this depth.
    chosen: UnitId,
    /// Units already explored from this prefix, each with the step it
    /// took [Abdulla2017 p:42:24 s:Algorithm 2 line 21].
    ///
    /// The step travels with the entry so [`inherit`] can test it at a
    /// depth where the unit never runs.
    sleep: BTreeMap<UnitId, StepFootprint>,
    /// Sequences the search still owes from this prefix.
    wut: WakeupTree,
    /// Footprint each unit produced when the search ran it from this
    /// prefix, which is what the independence test at line 17 reads.
    footprints: BTreeMap<UnitId, StepFootprint>,
    /// Sequences this frame already dropped, so a re-grafted sequence
    /// costs the reversal once.
    ///
    /// Every replay of the prefix above reaches the same state at this
    /// depth, so a branch it could not take once it cannot take again.
    /// [`Frame::drop_cost`] says what a sequence under a lost one costs.
    dropped: BTreeSet<Vec<UnitId>>,
    /// Units the all-blocked time warp woke here, empty at every depth
    /// no warp resolved.
    ///
    /// A replay of the prefix reaches the same warp, and that warp wakes
    /// the same set, so a later execution decides from the recorded set
    /// and the runtime's selection call after the warp delivers the
    /// choice.
    warp_woke: Vec<UnitId>,
}

impl Frame {
    /// What a drop of the branch through `head`, with `below` under it,
    /// costs the reversal count: one per sequence the branch carried
    /// that no sequence this frame already lost covers.
    ///
    /// A lost sequence covers:
    ///
    /// - itself;
    /// - every extension of it;
    /// - every prefix of it.
    ///
    /// That is the leaf rule of [`WakeupTree::insert`]: were the lost
    /// one deliverable, the tree would hold the other without a second
    /// branch.
    fn drop_cost(&mut self, head: UnitId, below: &WakeupTree) -> usize {
        let mut sequences = below.sequences();
        if sequences.is_empty() {
            sequences.push(Vec::new());
        }
        let mut cost = 0usize;
        for mut tail in sequences {
            tail.insert(0, head);
            let covered = self
                .dropped
                .iter()
                .any(|lost| lost.starts_with(&tail) || tail.starts_with(lost));
            if !covered {
                self.dropped.insert(tail);
                cost += 1;
            }
        }
        cost
    }
}

/// Why one execution of the search stopped.
enum Halt {
    /// The execution reached a maximal sequence or a bound.
    Stopped(StopReason),
    /// The search already explored every runnable unit from this
    /// prefix, so the run explored no class.
    SleepBlocked,
}

/// Run optimal DPOR on a workload.
///
/// The search calls `make_runtime` once, snapshots the runtime it
/// returns, and restores that snapshot per execution.
pub fn explore_optimal<F>(make_runtime: F, config: &ExplorationConfig) -> ExplorationResult
where
    F: FnMut() -> Runtime,
{
    explore_optimal_observed(make_runtime, config, |_, _| {})
}

/// [`explore_optimal`] that shows each execution's runtime to
/// `observe` before dropping it.
///
/// The search calls `observe` once per execution it records, in the
/// order [`ExplorationResult::schedules`] holds them, with `true` for
/// the baseline. A driver that captures memory regions per schedule
/// reads them here; the search reuses one runtime, so there is no later
/// chance.
pub fn explore_optimal_observed<F, O>(
    make_runtime: F,
    config: &ExplorationConfig,
    observe: O,
) -> ExplorationResult
where
    F: FnMut() -> Runtime,
    O: FnMut(&Runtime, bool),
{
    search(make_runtime, config, observe).0
}

/// [`explore_optimal_observed`] beside the site each dropped branch
/// came from ([`Drops`]).
fn search<F, O>(
    mut make_runtime: F,
    config: &ExplorationConfig,
    mut observe: O,
) -> (ExplorationResult, Drops)
where
    F: FnMut() -> Runtime,
    O: FnMut(&Runtime, bool),
{
    // One composition, one start state: a driver can hand over a
    // runtime it already advanced -- a window of a title boot -- and
    // the search has nothing to build a second one from.
    let mut rt = make_runtime();
    let start = rt.snapshot();
    let mut frames: Vec<Frame> = Vec::new();
    let mut schedules: Vec<ScheduleRecord> = Vec::new();
    let mut baseline: Option<BaselineRun> = None;
    let mut baseline_branching = 0usize;
    let mut first_invariant_break: Option<String> = None;
    let mut bounds_hit = false;
    let mut truncated_runs = 0usize;
    let mut refused_runs = 0usize;
    let mut sleep_blocked = 0usize;
    let mut dropped = Drops::default();
    // The depth the next execution re-decides at; the backtrack below
    // names it and `run_one` picks the unit there.
    let mut resume: Option<usize> = None;

    loop {
        let (run, halt) = run_one(&mut rt, &start, &mut frames, resume, config);
        let branch_step = resume;
        let alternate_choice = run.alternate_choice;
        dropped.add(run.dropped);
        if first_invariant_break.is_none() {
            first_invariant_break = run.invariant_break;
        }

        match halt {
            Halt::SleepBlocked => sleep_blocked += 1,
            Halt::Stopped(stop) => {
                let truncated = stop.is_truncated();
                // The two tallies below count alternates alone;
                // `mark_baseline_truncated` answers for the baseline.
                if truncated {
                    bounds_hit = true;
                }
                observe(&rt, branch_step.is_none());
                match branch_step {
                    Some(step) => {
                        let unit = alternate_choice
                            .expect("a resumed execution names the unit it re-decided with");
                        if truncated {
                            truncated_runs += 1;
                            if stop.class() == StopClass::Refusal {
                                refused_runs += 1;
                            }
                        }
                        schedules.push(ScheduleRecord {
                            branch_step: step,
                            alternate_choice: unit,
                            memory_hash: run.hash,
                            stop,
                            truncated,
                        });
                    }
                    None => {
                        baseline_branching = run.log.branching_count();
                        baseline = Some(BaselineRun {
                            hash: run.hash,
                            steps: run.log.len(),
                            stop,
                        });
                    }
                }
                // A truncated execution's race set covers a prefix of
                // the workload, so it owes nothing.
                if !truncated {
                    detect_races(&run.log, &mut frames);
                }
            }
        }

        if schedules.len() >= config.max_schedules {
            bounds_hit = true;
            break;
        }
        match backtrack(&mut frames) {
            Some(depth) => resume = Some(depth),
            None => break,
        }
    }

    let baseline = baseline.expect("the first execution is the baseline");
    let found_divergence = schedules
        .iter()
        .any(|record| !record.truncated && record.memory_hash != baseline.hash);
    let mut iter = AlternateIteration {
        schedules,
        bounds_hit,
        found_divergence,
        // A sleep-set block cuts a redundant execution, so it costs no
        // class.
        schedules_pruned: sleep_blocked,
        schedules_truncated: truncated_runs,
        schedules_refused: refused_runs,
    };
    if baseline.stop.is_truncated() {
        iter.mark_baseline_truncated();
    }
    // A sleep-set block explored no class and lost none, so it leaves
    // the count whole.
    let complete = !iter.bounds_hit && !baseline.stop.is_truncated() && dropped.total() == 0;
    let classes = iter.schedules.len().saturating_add(1);
    let mut result = classify_iteration(iter, baseline, baseline_branching, first_invariant_break);
    result.classes_explored = complete.then_some(classes);
    result.reversals_dropped = dropped.total();
    (result, dropped)
}

/// Run one execution and extend `frames` past the prefix they already
/// fix [Abdulla2017 p:42:24 s:Algorithm 2 lines 8-19].
fn run_one(
    rt: &mut Runtime,
    start: &RuntimeSnapshot,
    frames: &mut Vec<Frame>,
    resume: Option<usize>,
    config: &ExplorationConfig,
) -> (Run, Halt) {
    // Depths below this one replay the units they already chose.
    let fixed = resume.unwrap_or(0);
    rt.restore_into(start);
    let mut log = DecisionLog::new();
    let mut depth = 0usize;
    let mut dropped = Drops::default();
    let halt = loop {
        // Read before the cap: the restored snapshot can carry a
        // pending pass, and no footprint records the parks a step
        // under it runs behind.
        if rt.has_pending_child_init() {
            break Halt::Stopped(StopReason::ChildInitUnserved);
        }
        // An execution that reaches the cap with nothing left to run is
        // maximal, so the step below names its stop.
        let at_cap = depth >= config.max_steps_per_run;
        if at_cap && rt.can_take_another_step() {
            break Halt::Stopped(StopReason::StepBound);
        }
        let runnable: Vec<UnitId> = rt.registry().runnable_ids().collect();
        // With nothing runnable the runtime warps and picks; a depth
        // that recorded what its warp woke decides from that instead
        // (see the doc on `Frame::warp_woke`).
        let warp_woke: Vec<UnitId> = if runnable.is_empty() {
            frames
                .get(depth)
                .map(|frame| frame.warp_woke.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let deciding: &[UnitId] = if runnable.is_empty() {
            &warp_woke
        } else {
            &runnable
        };
        if !deciding.is_empty() && depth >= fixed {
            if depth == frames.len() {
                let (sleep, wut) = inherit(frames);
                frames.push(Frame {
                    chosen: deciding[0],
                    sleep,
                    wut,
                    footprints: BTreeMap::new(),
                    dropped: BTreeSet::new(),
                    warp_woke: Vec::new(),
                });
            }
            match choose(&mut frames[depth], deciding, &mut dropped.by_choose) {
                Some(unit) => frames[depth].chosen = unit,
                None => break Halt::SleepBlocked,
            }
        }
        if deciding.is_empty() {
            rt.set_scheduler(cellgov_core::RoundRobinScheduler::new());
        } else {
            // At a warp depth the first selection call finds nothing
            // runnable and advances no cursor (see the comment in
            // `PrescribedScheduler::select_next`), so the pass that
            // wakes a unit delivers the prescription.
            rt.set_scheduler(PrescribedScheduler::single_choice(frames[depth].chosen));
        }
        let step = match rt.step() {
            Ok(step) => step,
            Err(cellgov_core::StepError::NoRunnableUnit) => {
                break Halt::Stopped(StopReason::Stalled)
            }
            Err(cellgov_core::StepError::AllBlocked) => {
                break Halt::Stopped(StopReason::Deadlocked)
            }
            Err(e) => break Halt::Stopped(StopReason::StepError(e)),
        };
        // `can_take_another_step` and `Runtime::step` answer the same
        // question, so a step past the cap is the two disagreeing.
        debug_assert!(
            !at_cap,
            "the cap was reached, the predicate saw no step left, and one ran",
        );
        if runnable.is_empty() {
            if depth == frames.len() {
                let (sleep, wut) = inherit(frames);
                frames.push(Frame {
                    chosen: step.unit,
                    sleep,
                    wut,
                    footprints: BTreeMap::new(),
                    dropped: BTreeSet::new(),
                    warp_woke: Vec::new(),
                });
            }
            let frame = &mut frames[depth];
            // `last_runnable` is the set the scheduler chose from, so
            // it is the set the warp woke.
            let before = std::mem::replace(&mut frame.warp_woke, rt.last_runnable().to_vec());
            let decided = !before.is_empty();
            // The prefix below replays step for step and the warp runs
            // before the selection, so every visit reads the same set.
            debug_assert!(
                !decided || before == frame.warp_woke,
                "a replayed prefix reached the same warp and it woke {:?}, not {before:?}",
                frame.warp_woke,
            );
            // A decided visit prescribed `chosen` from the recorded
            // set, and a warp that delivered another unit would retire
            // a branch the frame still owes.
            debug_assert!(
                !decided || frame.chosen == step.unit,
                "the depth prescribed {:?} and the warp delivered {:?}",
                frame.chosen,
                step.unit,
            );
            // `inherit` hands almost every warp depth an empty tree: a
            // grafted branch names a second runnable unit, and a depth
            // holding one is no warp depth. A step that stops a unit it
            // does not name (`Runtime::handle_process_exit_child`
            // finishes every unit of the exiting pid) can leave a
            // branch here, and this loop retires what the warp cannot
            // deliver.
            if !decided || frame.chosen != step.unit {
                dropped.at_warp_visits += 1;
                for (head, below) in frame.wut.retain_branch(step.unit) {
                    dropped.at_warp += frame.drop_cost(head, &below);
                }
            }
            frame.chosen = step.unit;
        }
        let runnable = rt.last_runnable().to_vec();
        let chosen = frames[depth].chosen;
        let mut footprint =
            StepFootprint::from_step(step.unit, step.result.yield_reason, &step.effects);
        if let Err(e) = rt.commit_step(&step.result, &step.effects) {
            break Halt::Stopped(StopReason::CommitError(e));
        }
        footprint.note_commit(rt, step.unit);
        // The same stop as at the top of the loop, read after the
        // commit that can make the pass pending.
        if rt.has_pending_child_init() {
            break Halt::Stopped(StopReason::ChildInitUnserved);
        }
        // The commit discards and counts a faulted batch, so the fault
        // is read after it; a faulted step gets no decision point, like
        // a refused commit.
        if let Some(kind) = step.result.fault {
            break Halt::Stopped(StopReason::Faulted(kind));
        }
        debug_assert_eq!(
            step.unit, chosen,
            "the chosen unit came from the runnable set, so the scheduler took it",
        );
        frames[depth]
            .footprints
            .insert(step.unit, footprint.clone());
        log.push(DecisionPoint {
            step: depth,
            runnable,
            chosen: step.unit,
            footprint,
        });
        depth += 1;
    };
    // `Run::alternate_choice` says why this reads before the truncate.
    let alternate_choice = resume.and_then(|at| frames.get(at).map(|frame| frame.chosen));
    // A step that refused leaves the frame it pushed behind.
    frames.truncate(depth);
    let run = Run {
        hash: rt.observable_hash(),
        invariant_break: rt.lv2_host().observability().first_invariant_break_line(),
        log,
        alternate_choice,
        dropped,
    };
    (run, halt)
}

/// The sleep set and wakeup tree one depth inherits from the one above
/// [Abdulla2017 p:42:24 s:Algorithm 2 lines 17-18].
///
/// A parent with no footprint for its chosen unit wakes every sleeper,
/// which costs exploration and no soundness.
fn inherit(frames: &[Frame]) -> (BTreeMap<UnitId, StepFootprint>, WakeupTree) {
    let Some(parent) = frames.last() else {
        return (BTreeMap::new(), WakeupTree::new());
    };
    let wut = parent.wut.subtree(parent.chosen);
    let Some(taken) = parent.footprints.get(&parent.chosen) else {
        return (BTreeMap::new(), wut);
    };
    let sleep = parent
        .sleep
        .iter()
        .filter(|(_, asleep)| !asleep.conflicts(taken))
        .map(|(unit, asleep)| (*unit, asleep.clone()))
        .collect();
    (sleep, wut)
}

/// Read the races of a maximal execution and record what each one owes
/// [Abdulla2017 p:42:24 s:Algorithm 2 lines 2-7].
///
/// A depth a warp resolved takes a branch like any other; [`choose`]
/// drops what the warp cannot deliver.
fn detect_races(log: &DecisionLog, frames: &mut [Frame]) {
    let execution = Execution::from_log(log);
    let relation = execution.happens_before();
    let events = execution.events();
    let precedes = |first: usize, second: usize| {
        first < second && relation.precedes(events[first].id, events[second].id)
    };
    for race in execution.races(&relation) {
        let (at, second) = (race.first.index, race.second.index);
        // The sequence that reverses the race: everything between the
        // two that the earlier event does not hold back, then the later
        // event.
        let mut sequence: Vec<SeqEvent> = (at + 1..second)
            .filter(|index| !precedes(at, *index))
            .map(|index| SeqEvent {
                unit: events[index].id.unit,
                index,
            })
            .collect();
        sequence.push(SeqEvent {
            unit: race.second.unit,
            index: second,
        });
        let Some(frame) = frames.get_mut(at) else {
            continue;
        };
        if already_explored(&frame.sleep, &sequence, &precedes) {
            continue;
        }
        frame.wut.insert(&sequence, &precedes);
    }
}

/// True when the search already ran an execution equivalent to
/// `sequence` from this prefix, so the race that asked for it owes
/// nothing [Abdulla2017 p:42:24 s:Algorithm 2 line 6].
///
/// This reads one half of the weak-initials set
/// [Abdulla2017 p:42:12 s:Lemma 4.2]: a sleeping unit that can lead the
/// sequence. The other half, a sleeping unit whose own next step
/// commutes past the sequence, covers the sequence and not the subtree
/// under its branch; `tests/clock_read.rs` holds a workload where
/// reading it loses classes.
fn already_explored<F>(
    sleep: &BTreeMap<UnitId, StepFootprint>,
    sequence: &[SeqEvent],
    precedes: &F,
) -> bool
where
    F: Fn(usize, usize) -> bool,
{
    if sleep.is_empty() {
        return false;
    }
    let leaders = initials(sequence, precedes);
    sleep.keys().any(|unit| leaders.contains(unit))
}

/// Drop the branch each frame just explored, deepest first, and name
/// the depth the next execution re-decides at
/// [Abdulla2017 p:42:24 s:Algorithm 2 lines 20-21].
fn backtrack(frames: &mut Vec<Frame>) -> Option<usize> {
    while let Some(frame) = frames.last_mut() {
        let chosen = frame.chosen;
        frame.wut.remove_branch(chosen);
        if let Some(footprint) = frame.footprints.get(&chosen).cloned() {
            frame.sleep.insert(chosen, footprint);
        }
        if !frame.wut.is_empty() {
            return Some(frames.len() - 1);
        }
        frames.pop();
    }
    None
}

/// The unit one depth runs next [Abdulla2017 p:42:24 s:Algorithm 2
/// lines 11-16].
///
/// A branch whose unit is not runnable here names a reversal this state
/// cannot reach, since `Execution::races` does not ask; `choose` drops
/// it and [`Frame::drop_cost`] counts what it carried. With no branch
/// left the depth takes a runnable unit not asleep and makes it the
/// tree's one branch, so a later insert walks what this depth ran.
fn choose(frame: &mut Frame, runnable: &[UnitId], dropped: &mut usize) -> Option<UnitId> {
    while let Some(unit) = frame.wut.min_branch() {
        if runnable.contains(&unit) {
            return Some(unit);
        }
        let below = frame
            .wut
            .remove_branch(unit)
            .expect("min_branch named this branch, so the tree holds it");
        *dropped += frame.drop_cost(unit, &below);
    }
    let unit = runnable
        .iter()
        .find(|unit| !frame.sleep.contains_key(unit))
        .copied()?;
    frame.wut = WakeupTree::single(unit);
    Some(unit)
}

#[cfg(test)]
#[path = "tests/optimal_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/already_explored_tests.rs"]
mod already_explored_tests;

#[cfg(test)]
#[path = "tests/dropped_once_tests.rs"]
mod dropped_once_tests;

#[cfg(test)]
#[path = "tests/warp_retire_tests.rs"]
mod warp_retire_tests;
