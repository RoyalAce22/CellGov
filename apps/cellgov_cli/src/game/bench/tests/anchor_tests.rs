use cellgov_compare::BootOutcome;
use cellgov_time::Budget;

use super::super::options::SelectionArgs;
use super::super::test_fixtures::{
    anchor_fixture, bench_manifest, bench_options, measured_run, observed_stderr, test_cell,
    test_identity, TEST_CHECKPOINT,
};
use super::*;

#[test]
fn a_run_matching_its_anchor_reports_no_disagreements() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.is_empty(),
        "expected no failures, got {failures:?}"
    );
}

#[test]
fn an_exact_witness_that_moved_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(77, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("host_invariant_breaks") && failures[0].contains("77"),
        "failure must name the witness and the observed value: {}",
        failures[0]
    );
}

#[test]
fn an_at_least_witness_above_its_baseline_is_not_a_disagreement() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 9_999),
    );
    assert!(failures.is_empty(), "got {failures:?}");
}

#[test]
fn a_moved_step_count_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390100,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("steps 390100")),
        "got {failures:?}"
    );
}

#[test]
fn a_changed_outcome_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "ProcessExit",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("outcome ProcessExit")),
        "got {failures:?}"
    );
}

/// A stop condition the run never reaches leaves the step count, the
/// outcome and every witness intact, so nothing else in the comparison
/// sees a moved checkpoint.
#[test]
fn a_run_taken_at_another_checkpoint_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        manifest::CheckpointTrigger::Pc(0x1_0000),
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("Pc=0x10000") && failures[0].contains("ProcessExit"),
        "the failure must name both stop conditions: {}",
        failures[0]
    );
}

/// A moved budget retires a different trajectory under a step count
/// that did not move, and the recorded witnesses are at-least bounds
/// that do not catch it.
#[test]
fn a_run_at_another_budget_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(512),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("budget 512") && failures[0].contains("256"),
        "got {failures:?}"
    );
}

/// The steps and the witnesses come out of the measuring child, so the
/// triple must come out of that same stream.
#[test]
fn a_measured_run_that_named_no_triple_is_a_disagreement() {
    let root = crate::paths::workspace_root();
    let verdict = check_anchor_under(
        &root,
        "NPUA80001",
        &test_cell(),
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    let AnchorVerdict::Drift(failures) = verdict else {
        panic!("expected Drift, got {verdict:?}");
    };
    assert!(
        failures
            .iter()
            .any(|f| f.contains(RUN_IDENTITY_SENTINEL) && f.contains("no")),
        "got {failures:?}"
    );
}

/// A file copied from another cell reproduces every witness of its own
/// run, so only the embedded triple names the wrong cell.
#[test]
fn an_anchor_measured_against_another_firmware_reports_the_triple() {
    let mut ran = test_identity();
    ran.firmware.as_mut().expect("firmware half").version = "3.55".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded 4.93") && failures[0].contains("ran 3.55"),
        "got {failures:?}"
    );
}

#[test]
fn an_anchor_measured_against_another_title_version_reports_the_triple() {
    let mut ran = test_identity();
    ran.game.as_mut().expect("game half").version = "update:02.51".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded CG_TEST base")
            && failures[0].contains("ran CG_TEST update:02.51"),
        "got {failures:?}"
    );
}

/// A reinstall from another PUP keeps the console-visible version, so
/// the report carries every compared field.
#[test]
fn two_firmwares_sharing_a_version_are_still_told_apart_in_the_report() {
    let mut ran = test_identity();
    ran.firmware.as_mut().expect("firmware half").pup_sha256 = "ff".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("pup sha256 00") && failures[0].contains("pup sha256 ff"),
        "got {failures:?}"
    );
}

/// The same for the game half: two trees of one version can differ in
/// the `APP_VER` their PARAM.SFO declares.
#[test]
fn two_title_trees_sharing_a_version_are_still_told_apart_in_the_report() {
    let mut ran = test_identity();
    ran.game.as_mut().expect("game half").app_ver = "01.01".to_string();
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("app_ver 01.00") && failures[0].contains("app_ver 01.01"),
        "got {failures:?}"
    );
}

/// An anchor can predate the install of one half of the triple.
#[test]
fn a_half_the_anchor_never_named_is_reported_as_unidentified() {
    let mut ran = test_identity();
    ran.game = None;
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(failures[0].contains("(unidentified)"), "got {failures:?}");
}

/// `FromStr` round-trips the Display form, so the comparison must use
/// it too: Debug renders the address in decimal and would report a
/// mismatch against an identical outcome.
#[test]
fn a_pc_reached_outcome_compares_by_its_display_form() {
    let mut baseline = anchor_fixture(73);
    baseline.outcome = BootOutcome::PcReached(0x1_0000);
    let observed = observed_stderr(73, 100);
    let same = anchor_disagreements(
        &baseline,
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "PcReached(0x10000)",
        &observed,
    );
    assert!(same.is_empty(), "identical outcome must match: {same:?}");
    let debug_form = anchor_disagreements(
        &baseline,
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "PcReached(65536)",
        &observed,
    );
    assert!(
        !debug_form.is_empty(),
        "the decimal Debug form is not equal"
    );
}

#[test]
fn a_cell_with_no_committed_anchor_is_skipped_not_failed() {
    let verdict = check_anchor(
        "CG_NO_SUCH_CONTENT_ID",
        &test_cell(),
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    assert_eq!(verdict, AnchorVerdict::NotRecorded("fw 4.93 x base".into()));
}

/// The anchor tree is keyed by the whole triple, so a file one path
/// segment away is another cell's anchor.
#[test]
fn a_sibling_cells_anchor_does_not_stand_in_for_an_unrecorded_one() {
    let other = CellKey {
        fw: "3.55".to_string(),
        game_ver: Some("base".to_string()),
    };
    let verdict = check_anchor(
        "NPUA80001",
        &other,
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    assert_eq!(verdict, AnchorVerdict::NotRecorded("fw 3.55 x base".into()));
}

/// The workspace root is compiled in, so every cell looks unrecorded
/// once the binary leaves its source tree. Saying so is the difference
/// between a reported skip and a gate that quietly stopped gating.
#[test]
fn an_unreachable_workspace_root_does_not_read_as_an_unrecorded_cell() {
    let absent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("no_such_workspace_root");
    let verdict = check_anchor_under(
        &absent,
        "VSH",
        &test_cell(),
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    let AnchorVerdict::NotComparable(reasons) = verdict else {
        panic!("expected NotComparable, got {verdict:?}");
    };
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(
        reasons[0].contains("no_such_workspace_root"),
        "the reason must name the path it looked under: {}",
        reasons[0]
    );
}

#[test]
fn an_anchor_with_no_witnesses_is_a_disagreement() {
    let mut baseline = anchor_fixture(73);
    baseline.witnesses.clear();
    let failures = anchor_disagreements(
        &baseline,
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert_eq!(failures, vec!["anchor records no witnesses".to_string()]);
}

#[test]
fn a_recorded_witness_whose_line_never_appeared_is_reported() {
    let observed = parse_witness_lines("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n")
        .expect("synthetic witness line parses");
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed,
    );
    assert!(
        failures
            .iter()
            .any(|f| f.contains("ldarx") && f.contains("BENCH_ATOMIC_WITNESS:")),
        "a missing emitter must not read as an observed zero: {failures:?}"
    );
}

#[test]
fn a_witness_the_anchor_does_not_carry_is_reported() {
    let observed = parse_witness_lines(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n\
         BENCH_ATOMIC_WITNESS: ldarx=100 stdcx=0 lwarx=0 stwcx=0\n\
         BENCH_DCBZ_WITNESS: count=4\n",
    )
    .expect("synthetic witness lines parse");
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed,
    );
    assert_eq!(
        failures,
        vec!["witness dcbz is emitted but not recorded in the anchor".to_string()]
    );
}

#[test]
fn a_zero_step_run_against_a_recorded_anchor_is_a_disagreement() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        &test_identity(),
        TEST_CHECKPOINT,
        0,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("steps 0")),
        "got {failures:?}"
    );
}

#[test]
fn a_run_at_the_recorded_cap_is_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    assert!(incomparable_reasons(&bench_options(&title, Some(&cell), &[])).is_empty());
    let capped = bench_manifest(Some(4_000));
    let opts = bench_options(&capped, Some(&cell), &[]);
    assert_eq!(opts.max_steps, 4_000);
    assert!(incomparable_reasons(&opts).is_empty());
}

/// A diagnostic-only flag must not disable the gate: `--prescan` only
/// prints a decode report before execution.
#[test]
fn prescan_leaves_the_run_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.prescan = true;
    assert!(incomparable_reasons(&opts).is_empty());
}

#[test]
fn a_shortened_run_is_not_compared_against_the_anchor() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.max_steps = 50_000;
    let reasons = incomparable_reasons(&opts);
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(reasons[0].contains("--max-steps 50000"), "got {reasons:?}");
}

/// The cell's cap is what its anchor was recorded at, so a run at the
/// title-level default is the retargeted one.
#[test]
fn a_run_at_the_title_cap_is_incomparable_against_a_cell_that_overrides_it() {
    let cell = test_cell();
    let title = bench_manifest(Some(4_000));
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.plan.max_steps = 250;
    let reasons = incomparable_reasons(&opts);
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(
        reasons[0].contains("--max-steps 4000") && reasons[0].contains("250"),
        "got {reasons:?}"
    );
}

#[test]
fn every_trajectory_override_names_itself_as_incomparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let args = vec!["EBOOT.BIN".to_string()];

    let mut checkpoint = bench_options(&title, Some(&cell), &[]);
    checkpoint.checkpoint_override = Some(manifest::CheckpointTrigger::Pc(0x1_0000));
    let mut budget = bench_options(&title, Some(&cell), &[]);
    budget.budget_override = Some(Budget::new(512));
    let mut strict = bench_options(&title, Some(&cell), &[]);
    strict.strict_reserved = true;
    let guest = bench_options(&title, Some(&cell), &args);

    for (label, opts) in [
        ("--checkpoint", checkpoint),
        ("--budget", budget),
        ("--strict-reserved", strict),
        ("--guest-arg", guest),
    ] {
        let reasons = incomparable_reasons(&opts);
        assert_eq!(reasons.len(), 1, "{label}: got {reasons:?}");
        assert!(reasons[0].contains(label), "{label}: got {reasons:?}");
    }
}

/// An anchor is filed under a cell, so a run that composed none has
/// nothing to be held against.
#[test]
fn a_run_that_composed_no_cell_is_not_compared() {
    let title = bench_manifest(None);

    let mut unmanaged = bench_options(&title, None, &[]);
    unmanaged.selection = SelectionArgs {
        firmware_dir: Some("dev_flash/sys/external"),
        ..SelectionArgs::default()
    };
    let reasons = incomparable_reasons(&unmanaged);
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(reasons[0].contains("--firmware-dir"), "got {reasons:?}");

    let reasons = incomparable_reasons(&bench_options(&title, None, &[]));
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(reasons[0].contains("composed no cell"), "got {reasons:?}");
}

/// The anchor is keyed by the firmware and the game version, so a run
/// that selects them composes the cell it is held against.
#[test]
fn selecting_a_firmware_and_a_game_version_leaves_the_run_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.selection = SelectionArgs {
        fw: Some("4.93"),
        game_ver: Some("base"),
        ..SelectionArgs::default()
    };
    assert!(incomparable_reasons(&opts).is_empty());
}

/// Restating the cell's own checkpoint is not a retarget, so it must
/// not disable the comparison.
#[test]
fn a_checkpoint_override_equal_to_the_cells_stays_comparable() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.checkpoint_override = Some(opts.plan.checkpoint);
    assert!(incomparable_reasons(&opts).is_empty());
}
