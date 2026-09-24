use cellgov_compare::RUN_IDENTITY_SENTINEL;
use cellgov_time::Budget;

use super::super::options::SelectionArgs;
use super::super::test_fixtures::{bench_manifest, bench_options, measured_run, test_cell};
use super::*;
use cellgov_boot::manifest;

/// The steps and the witnesses come from the measuring child, so the
/// identity triple must come from that same stream.
#[test]
fn a_measured_run_that_named_no_triple_is_a_disagreement() {
    let root = crate::paths::workspace_root();
    let verdict = check_anchor_under(
        &root,
        "BCES00664",
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

#[test]
fn a_cell_with_no_committed_anchor_is_skipped_not_failed() {
    let verdict = check_anchor(
        "CG_NO_SUCH_CONTENT_ID",
        &test_cell(),
        &measured_run("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n"),
    );
    assert_eq!(verdict, AnchorVerdict::NotRecorded("fw 4.93 x base".into()));
}

/// The whole identity triple keys the anchor tree, so a file one path
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
