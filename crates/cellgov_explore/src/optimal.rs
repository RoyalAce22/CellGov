//! Optimal DPOR: source sets and wakeup trees
//! [Abdulla2017 p:42:24 s:Algorithm 2].
//!
//! The search runs one execution to a maximal sequence, then reads its
//! races. A race asks for an execution where the later event runs
//! first, and the sequence that reaches it goes into the wakeup tree at
//! the prefix before the earlier event. The next execution takes the
//! least branch that tree holds.
//!
//! A sleep set carries the units already explored from a prefix, so the
//! search does not run one twice. The wakeup tree keeps the sleep set
//! from blocking; [`crate::wakeup`] says how.
//!
//! Each maximal execution the search runs stands for one equivalence
//! class of schedules.

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
    /// `run_one` reads this before it cuts the frame stack back to
    /// the steps that committed. A first step the model refuses cuts
    /// that frame away, and the record still names the alternate the
    /// run took.
    alternate_choice: Option<UnitId>,
    dropped: Drops,
}

/// Wakeup-tree branches the search gave up, by the site that gave each
/// one up.
///
/// One aggregate cannot say which site a run's drops came from, and the
/// two sites reach a branch for different reasons. The public
/// [`ExplorationResult::reversals_dropped`] carries [`Drops::total`],
/// which is the two drop counts and not the visit count beside them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Drops {
    /// [`choose`] met a branch whose head cannot run at the depth
    /// holding it.
    by_choose: usize,
    /// A warp depth met an inherited branch, and the warp woke a
    /// different unit ([`Frame::warp_woke`]).
    at_warp: usize,
    /// Visits at which a warp depth read its own tree, which are the
    /// only visits `at_warp` can move on.
    ///
    /// A run that reaches no warp depth leaves `at_warp` at zero
    /// whatever that site would answer, so a reader of `at_warp` needs
    /// this beside it.
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
    /// The step travels with the entry: a unit that sleeps down a level
    /// never runs there, and the independence test at line 17 still
    /// reads its step.
    sleep: BTreeMap<UnitId, StepFootprint>,
    /// Sequences the search still owes from this prefix.
    wut: WakeupTree,
    /// Footprint each unit produced when the search ran it from this
    /// prefix, which is what the independence test at line 17 reads.
    footprints: BTreeMap<UnitId, StepFootprint>,
    /// Sequences this frame already dropped, so a re-grafted sequence
    /// costs the reversal once.
    ///
    /// A race can graft a sequence under a head this frame dropped
    /// earlier, and the state at this depth is what every replay of the
    /// prefix above reaches, so the branch is unreachable again. Two
    /// races can graft different tails under one head, and each is cover
    /// the frame lost. An extension or a prefix of a lost sequence costs
    /// nothing; [`Frame::drop_cost`] says why.
    /// [`crate::classify::ExplorationResult::reversals_dropped`] says
    /// what the count this feeds does and does not measure.
    dropped: BTreeSet<Vec<UnitId>>,
    /// Units the all-blocked time warp woke here, empty at every depth
    /// no warp resolved.
    ///
    /// No unit is runnable at such a depth, so the search has nothing
    /// to decide from and lets the runtime warp guest time and pick. A
    /// replay of the same prefix reaches the same warp, so that warp
    /// wakes the same set. A later execution decides from the recorded
    /// set, and the selection call the runtime makes after the warp
    /// delivers that choice.
    warp_woke: Vec<UnitId>,
}

impl Frame {
    /// What a drop of the branch through `head`, with `below` under it,
    /// costs the reversal count. Each sequence the branch carried costs
    /// one, unless a sequence this frame already lost covers it.
    ///
    /// A lost sequence covers:
    ///
    /// - itself;
    /// - every extension of it;
    /// - every prefix of it.
    ///
    /// That is how [`WakeupTree::insert`] treats a leaf: with the lost
    /// one deliverable, the tree holds the other without a second
    /// branch. A head with nothing below it carries the one-unit
    /// sequence.
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
///
/// `config.max_schedules` bounds the equivalence classes explored and
/// `config.max_steps_per_run` bounds each execution.
///
/// [`ExplorationResult::schedules`] holds every execution after the
/// first; the first is the baseline. So the classes explored are
/// `schedules.len() + 1` whenever the baseline execution was maximal.
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
/// came from.
///
/// The result carries one aggregate, so a test that means to reach one
/// of the two sites cannot tell from it which site answered.
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
                // A truncated execution bounds the search whichever run
                // it was. The two tallies below count alternates alone,
                // because that is what they answer for and what
                // `mark_baseline_truncated` rewrites one of them to.
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
        // A sleep set holds the units whose every extension the search
        // already explored. A block cuts a redundant execution, so it
        // costs no class.
        schedules_pruned: sleep_blocked,
        schedules_truncated: truncated_runs,
        schedules_refused: refused_runs,
    };
    if baseline.stop.is_truncated() {
        iter.mark_baseline_truncated();
    }
    // Every execution the search ran stands for one class. A bound or a
    // dropped branch stops the search before it covers every class; a
    // sleep-set block leaves the count whole.
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
        // Read before the cap: a window nothing can model is no
        // caller's bound. The snapshot every execution restores can
        // carry a pending pass, and the step below would then run under
        // parks no footprint records -- in every execution the search
        // runs, not one.
        if rt.has_pending_child_init() {
            break Halt::Stopped(StopReason::ChildInitUnserved);
        }
        // The cap refuses to start a step, so it answers only where
        // there was one to start. An execution that reaches the cap
        // with nothing left to run is maximal, and the step below
        // names the stop its work justifies.
        let at_cap = depth >= config.max_steps_per_run;
        if at_cap && rt.can_take_another_step() {
            break Halt::Stopped(StopReason::StepBound);
        }
        let runnable: Vec<UnitId> = rt.registry().runnable_ids().collect();
        // An empty set is no choice the search can make yet: the
        // runtime warps guest time, fires what is due, and schedules
        // whatever that wakes. A depth that already recorded the set a
        // warp woke decides from that instead.
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
            // At a warp depth the runtime asks once before the warp,
            // and again after each pass of it. Nothing is runnable at
            // the first call, and an unanswered call advances no
            // cursor, so the pass that wakes someone delivers the
            // prescription.
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
        // The predicate is a second reading of the question `Runtime::step`
        // itself answers. A step that runs past the cap is the two
        // disagreeing, and the cap then bounds nothing.
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
            // The search replays the prefix below step for step, and
            // the warp runs before the selection, so both visits read
            // the same set. A visit that read a different one decided
            // from a set that no longer describes this depth.
            debug_assert!(
                !decided || before == frame.warp_woke,
                "a replayed prefix reached the same warp and it woke {:?}, not {before:?}",
                frame.warp_woke,
            );
            // Where the depth already knew the set, `choose` picked out
            // of it above and the runtime delivered that pick. Where it
            // did not, this execution took whatever the warp gave. The
            // next execution to reach this depth decides from the set
            // this one recorded.
            //
            // A decided visit that retired a branch would retire one the
            // frame may still owe. The memo would then cost nothing for
            // the second drop of each sequence it carried.
            debug_assert!(
                !decided || frame.chosen == step.unit,
                "the depth prescribed {:?} and the warp delivered {:?}",
                frame.chosen,
                step.unit,
            );
            // A first visit arms the depth from what `inherit` handed
            // down, and that is empty at almost every warp depth. A
            // non-empty one needs the depth above to hold a branch
            // through its own chosen unit with a continuation. Only a
            // grafted sequence puts one there, and program order keeps
            // the unit that ran at a depth out of every sequence a race
            // grafts on it. The graft's head is the sequence's own first
            // event, which leads it, so a branch of that head would have
            // taken the walk instead (`WakeupTree::insert`). The head
            // names a second unit, runnable where the branch sits, and
            // no step parks a unit that was already runnable. A run that
            // takes the branch therefore leaves that second unit
            // runnable here, and a depth holding a runnable unit is no
            // warp depth.
            //
            // The gap is a step that stops a unit it does not name:
            // `Runtime::handle_process_exit_child` finishes every unit
            // of the exiting pid, so a workload that exits a child
            // process can reach this depth holding a branch. The loop
            // below answers for that, and `at_warp_visits` says how
            // often the loop read a tree at all.
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
        // The pass this parks behind reaches no footprint, so the
        // relation cannot answer for the steps after it.
        if rt.has_pending_child_init() {
            break Halt::Stopped(StopReason::ChildInitUnserved);
        }
        // The commit is what discards a faulted batch and counts it, so
        // the break reads the fault after it. The step gets no decision
        // point, as a refused commit gets none.
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
/// A unit stays asleep only where the step it took from the parent
/// prefix is independent of the step the parent ran. A unit the search
/// never ran from that prefix has no footprint to test, so it wakes:
/// that costs exploration, never soundness.
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
/// A depth a warp resolved takes a branch like any other. [`choose`]
/// asks whether the warp can deliver it, against [`Frame::warp_woke`],
/// and drops the branch it cannot.
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
        // event. Nothing in the window happens after the later event,
        // so the whole sequence can run before the earlier one.
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
/// sequence. The half for a sleeping unit whose own next step commutes
/// past the sequence is not read.
///
/// A branch does two things: it covers the sequence, and it opens the
/// subtree under it. The commuting half answers for the first alone, so
/// reading it retires branches whose descendants nothing else reaches;
/// `tests/clock_read.rs` holds a workload where that loses classes. The
/// leading half alone inserts a branch the search may not owe, which
/// costs exploration and no cover.
///
/// `sleep` carries a footprint per entry for the independence test at
/// line 17; this reads its units alone.
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
/// The wakeup tree's least branch goes first. A branch whose unit is
/// not runnable here names a reversal this state cannot reach:
/// `Execution::races` reports a racing pair without asking whether the
/// later unit can go first here. `choose` drops that branch and counts
/// it in `dropped`, once for each sequence the branch carried that no
/// earlier drop at this frame covers ([`Frame::drop_cost`]). Each
/// sequence is an owed execution, so a drop gives up at least one
/// equivalence class, which is why a count above zero withdraws
/// [`ExplorationResult::classes_explored`].
///
/// With no branch left the depth takes any runnable unit it did not
/// explore and records that choice as the tree's one branch, so a race
/// the execution reports inserts against a tree that already holds what
/// this depth ran. With none of those `choose` returns `None`.
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
