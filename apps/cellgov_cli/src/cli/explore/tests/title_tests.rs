//! The window a title verdict names, and the status it exits with.

use super::*;
use cellgov_core::{CommitError, StepError};
use cellgov_effects::FaultKind;
use cellgov_explore::ScheduleRecord;
use cellgov_mem::MemError;

/// The commit refusal a `first-rsx-write` cell stops at.
const RSX_CHECKPOINT_WRITE: CommitError = CommitError::Memory(MemError::ReservedWrite {
    addr: 0x0C00_0000,
    region: "rsx",
});

/// A refused commit that names no checkpoint.
const OTHER_REFUSAL: CommitError = CommitError::OutOfRange { effect_index: 0 };

fn alternate(stop: StopReason) -> ScheduleRecord {
    ScheduleRecord {
        branch_step: 4,
        alternate_choice: cellgov_event::UnitId::new(1),
        memory_hash: 0xCAFE_BABE,
        stop,
        truncated: stop.is_truncated(),
    }
}

fn result(stop: StopReason, alternates: Vec<ScheduleRecord>) -> ExplorationResult {
    let refused = alternates
        .iter()
        .filter(|s| s.stop.class() == StopClass::Refusal)
        .count();
    ExplorationResult {
        baseline_hash: 0xDEAD_BEEF,
        baseline_steps: 500,
        baseline_stop: stop,
        schedules_truncated: alternates.iter().filter(|s| s.truncated).count(),
        schedules: alternates,
        outcome: OutcomeClass::ScheduleStable,
        total_branching_points: 3,
        classes_explored: None,
        reversals_dropped: 0,
        bounds_hit: false,
        schedules_pruned: 0,
        schedules_refused: refused,
        first_invariant_break: None,
    }
}

/// A registry name no title claims: these cases render a report and
/// read no corpus, so naming an installed title would imply one.
const SAMPLE_TITLE: &str = "sample-title";

fn window(checkpoint: CheckpointTrigger) -> Window {
    Window {
        title: SAMPLE_TITLE.to_string(),
        start: WindowStart::FirstBranchingPoint,
        opened_at: 1_200,
        checkpoint,
    }
}

#[test]
fn the_window_runs_from_where_it_opened_to_the_baseline_s_last_step() {
    let r = result(StopReason::Stalled, vec![]);
    let w = window(CheckpointTrigger::ProcessExit);
    assert_eq!(w.closed_at(&r), 1_700);
    let text = human(&w, &r);
    assert!(text.contains("window: steps 1200..1700"), "{text}");
    assert!(
        text.contains("window_start: first branching point (step 1200)"),
        "{text}"
    );
}

#[test]
fn the_step_a_refused_commit_took_is_inside_the_window_it_did_not_commit_to() {
    // `baseline_steps` counts committed steps, and the RSX checkpoint
    // arrives as a commit the pipeline refused. The window reaches the
    // step that was taken; the hash covers one fewer.
    let w = window(CheckpointTrigger::FirstRsxWrite);
    let refused = result(StopReason::CommitError(RSX_CHECKPOINT_WRITE), vec![]);
    assert_eq!(
        w.closed_at(&refused),
        1_701,
        "the refused step was taken, so the window reaches it",
    );
    assert!(human(&w, &refused).contains("window: steps 1200..1701"));

    // A step `Runtime::step` itself refused was never taken, so the
    // window ends where the commits did.
    let capped = result(StopReason::StepError(StepError::MaxStepsExceeded), vec![]);
    assert_eq!(w.closed_at(&capped), 1_700);
    assert_eq!(w.closed_at(&result(StopReason::Stalled, vec![])), 1_700);
}

#[test]
fn a_step_count_start_is_not_repeated_back_as_the_step_it_opened_at() {
    let r = result(StopReason::Stalled, vec![]);
    let w = Window {
        start: WindowStart::Step(1_200),
        ..window(CheckpointTrigger::ProcessExit)
    };
    assert!(
        human(&w, &r).contains("window_start: step 1200\n"),
        "{}",
        human(&w, &r)
    );
}

#[test]
fn a_window_with_no_choice_in_it_says_so() {
    let mut r = result(StopReason::Stalled, vec![]);
    let w = window(CheckpointTrigger::ProcessExit);
    r.total_branching_points = 0;
    assert!(human(&w, &r).contains("one schedule"));
    assert!(
        !human(&w, &result(StopReason::Stalled, vec![])).contains("one schedule"),
        "the note belongs only to a window that held no branching point",
    );
}

#[test]
fn the_json_document_names_the_window_beside_the_exploration() {
    let r = result(StopReason::Stalled, vec![]);
    let v: serde_json::Value =
        serde_json::from_str(&json(&window(CheckpointTrigger::ProcessExit), &r).unwrap())
            .expect("the document parses");
    assert_eq!(v["title"], SAMPLE_TITLE);
    assert_eq!(v["window"]["opened_at"], 1_200);
    assert_eq!(v["window"]["closed_at"], 1_700);
    assert_eq!(v["window"]["start"], "first branching point");
    assert_eq!(v["window"]["ended_at_checkpoint"], serde_json::Value::Null);
    assert_eq!(v["model_refusals"], 0);
    assert_eq!(v["exploration"]["outcome"], "schedule-stable");
}

#[test]
fn a_bound_that_stopped_the_baseline_exits_clean() {
    // The caller set the cap, so reaching it is inconclusive rather
    // than a defect to chase.
    let r = result(
        StopReason::StepError(StepError::MaxStepsExceeded),
        vec![alternate(StopReason::StepBound)],
    );
    let w = window(CheckpointTrigger::ProcessExit);
    assert_eq!(w.model_refusals(&r), 0);
    assert_eq!(exit_code(&w, &r), 0);
}

#[test]
fn a_refused_schedule_outranks_the_verdict_it_leaves_behind() {
    let w = window(CheckpointTrigger::ProcessExit);

    let refused_alternate = result(
        StopReason::Stalled,
        vec![alternate(StopReason::CommitError(OTHER_REFUSAL))],
    );
    assert_eq!(w.model_refusals(&refused_alternate), 1);
    assert_eq!(exit_code(&w, &refused_alternate), EXIT_MODEL_REFUSAL);

    let refused_baseline = result(StopReason::CommitError(OTHER_REFUSAL), vec![]);
    assert_eq!(exit_code(&w, &refused_baseline), EXIT_MODEL_REFUSAL);

    let mut sensitive_but_refused = refused_alternate;
    sensitive_but_refused.outcome = OutcomeClass::ScheduleSensitive;
    assert_eq!(
        exit_code(&w, &sensitive_but_refused),
        EXIT_MODEL_REFUSAL,
        "a schedule the model would not run leaves the verdict resting on the rest",
    );
}

/// A guest fault exits non-zero, and not under the refusal's name.
///
/// The two say different things: a refusal is the model declining a
/// step, a fault is the guest's own step failing. `explore window`
/// already called this stop a fault, and a reader comparing the two
/// entry points on one boot would have seen two names for it.
#[test]
fn a_guest_fault_exits_under_its_own_name() {
    let w = window(CheckpointTrigger::ProcessExit);
    let fault = StopReason::Faulted(FaultKind::Guest(7));

    let faulted_alternate = result(StopReason::Stalled, vec![alternate(fault)]);
    assert_eq!(
        w.model_refusals(&faulted_alternate),
        0,
        "the model refused nothing",
    );
    assert_eq!(w.guest_faults(&faulted_alternate), 1);
    assert_eq!(exit_code(&w, &faulted_alternate), EXIT_GUEST_FAULT);
    assert_ne!(EXIT_GUEST_FAULT, EXIT_MODEL_REFUSAL);
    assert_ne!(EXIT_GUEST_FAULT, 0, "a faulted exploration is a finding");

    let faulted_baseline = result(fault, vec![]);
    assert_eq!(exit_code(&w, &faulted_baseline), EXIT_GUEST_FAULT);

    let mut sensitive_but_faulted = faulted_alternate;
    sensitive_but_faulted.outcome = OutcomeClass::ScheduleSensitive;
    assert_eq!(
        exit_code(&w, &sensitive_but_faulted),
        EXIT_GUEST_FAULT,
        "a schedule that faulted leaves the verdict resting on the rest",
    );
}

/// An unserved window exits non-zero, and not as a model refusal.
///
/// Splitting the class without giving the CLI an arm would drop
/// `model_refusals` to zero and exit clean, which reads as a verdict
/// over a window that reached none.
#[test]
fn an_unserved_window_exits_under_its_own_name() {
    let w = window(CheckpointTrigger::ProcessExit);
    let r = result(StopReason::ChildInitUnserved, vec![]);

    assert_eq!(w.model_refusals(&r), 0, "the model refused nothing");
    assert_eq!(w.guest_faults(&r), 0, "and the guest faulted on nothing");
    assert_eq!(w.unserved(&r), 1);
    assert_eq!(exit_code(&w, &r), EXIT_WINDOW_UNSERVED);
    assert_ne!(EXIT_WINDOW_UNSERVED, 0, "no verdict rests on this window");
    assert_ne!(EXIT_WINDOW_UNSERVED, EXIT_MODEL_REFUSAL);
    assert_ne!(EXIT_WINDOW_UNSERVED, EXIT_GUEST_FAULT);

    let text = human(&w, &r);
    assert!(text.contains("unserved: 1"), "{text}");
    assert!(
        text.contains("start the window after the spawn"),
        "the reading carries a remedy, as the never-opened one does: {text}",
    );

    let mut sensitive_but_unserved = r;
    sensitive_but_unserved.outcome = OutcomeClass::ScheduleSensitive;
    assert_eq!(
        exit_code(&w, &sensitive_but_unserved),
        EXIT_WINDOW_UNSERVED,
        "an outcome computed over a window the search would not answer for is not a verdict",
    );
}

/// A refusal and a fault in one run: the refusal is the finding.
#[test]
fn a_model_refusal_outranks_a_guest_fault() {
    let w = window(CheckpointTrigger::ProcessExit);
    let both = result(
        StopReason::Stalled,
        vec![
            alternate(StopReason::Faulted(FaultKind::Guest(7))),
            alternate(StopReason::CommitError(OTHER_REFUSAL)),
        ],
    );
    assert_eq!(w.guest_faults(&both), 1);
    assert_eq!(w.model_refusals(&both), 1);
    assert_eq!(
        exit_code(&w, &both),
        EXIT_MODEL_REFUSAL,
        "a defect in the model outranks the guest's own step failing",
    );
}

/// The count the guest-fault status comes from reaches both reports.
///
/// The exploration's own tallies count refusals and truncations, so the
/// window's lines are where a reader checks the status against a number.
#[test]
fn the_reports_name_the_fault_count_the_status_came_from() {
    let w = window(CheckpointTrigger::ProcessExit);
    let faulted = result(
        StopReason::Stalled,
        vec![alternate(StopReason::Faulted(FaultKind::Guest(7)))],
    );

    let text = human(&w, &faulted);
    assert!(text.contains("guest_faults: 1"), "{text}");
    assert!(
        text.contains("(fault)"),
        "the schedule line names the class, and `explore window` prints the same word: {text}",
    );

    let v: serde_json::Value =
        serde_json::from_str(&json(&w, &faulted).unwrap()).expect("the document parses");
    assert_eq!(v["guest_faults"], 1);
    assert_eq!(v["model_refusals"], 0);
    assert_eq!(v["exploration"]["schedules"][0]["stop_class"], "fault");

    let clean = result(StopReason::Stalled, vec![alternate(StopReason::Stalled)]);
    assert!(
        human(&w, &clean).contains("guest_faults: 0"),
        "the line is a measurement, so it prints where nothing faulted",
    );
}

#[test]
fn the_cell_s_rsx_checkpoint_is_not_a_model_refusal() {
    // A `first-rsx-write` cell stops on a refused write into the
    // reserved RSX region, which is the shape a pipeline defect takes
    // as well. Only the cell's own trigger tells the two apart.
    let r = result(
        StopReason::CommitError(RSX_CHECKPOINT_WRITE),
        vec![alternate(StopReason::CommitError(RSX_CHECKPOINT_WRITE))],
    );
    let at_checkpoint = window(CheckpointTrigger::FirstRsxWrite);
    assert_eq!(at_checkpoint.model_refusals(&r), 0);
    assert_eq!(exit_code(&at_checkpoint, &r), 0);
    let text = human(&at_checkpoint, &r);
    assert!(
        text.contains("window_end: the cell's first-rsx-write"),
        "{text}"
    );
    assert!(text.contains("0x0c000000"), "{text}");

    // The same refusal in a cell that stops elsewhere is a defect.
    let elsewhere = window(CheckpointTrigger::ProcessExit);
    assert_eq!(elsewhere.model_refusals(&r), 2);
    assert_eq!(exit_code(&elsewhere, &r), EXIT_MODEL_REFUSAL);
    assert!(!human(&elsewhere, &r).contains("window_end:"));
}

#[test]
fn a_sensitive_window_exits_the_way_the_scenario_and_microtest_targets_do() {
    let mut r = result(StopReason::Stalled, vec![]);
    r.outcome = OutcomeClass::ScheduleSensitive;
    assert_eq!(
        exit_code(&window(CheckpointTrigger::ProcessExit), &r),
        EXIT_SCHEDULE_SENSITIVE
    );
    assert_eq!(EXIT_SCHEDULE_SENSITIVE, 1);
}

#[test]
fn a_stable_window_exits_clean() {
    let r = result(StopReason::Stalled, vec![]);
    assert_eq!(exit_code(&window(CheckpointTrigger::ProcessExit), &r), 0);
}

#[test]
fn a_window_the_alternates_stopped_short_of_does_not_read_as_covered() {
    // `--max-steps-per-run` bounds each replay on its own, so an
    // alternate can stop well inside the range the window names.
    let w = window(CheckpointTrigger::ProcessExit);
    let bounded = result(StopReason::Stalled, vec![alternate(StopReason::StepBound)]);
    assert_eq!(bounded.schedules_truncated, 1);
    let text = human(&w, &bounded);
    assert!(
        text.contains("window_covered: 1 of 1 alternate(s) stopped before the window closed"),
        "{text}"
    );

    let covered = result(StopReason::Stalled, vec![alternate(StopReason::Stalled)]);
    assert!(
        !human(&w, &covered).contains("window_covered:"),
        "a window every alternate ran to the end of needs no coverage line",
    );
}
