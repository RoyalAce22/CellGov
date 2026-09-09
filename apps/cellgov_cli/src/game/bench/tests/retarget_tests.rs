//! Which overrides move a bench run off its anchor's trajectory, and
//! how that agrees with the gate's own list.

use cellgov_time::Budget;

use super::super::anchor::incomparable_reasons;
use super::super::test_fixtures::{bench_manifest, bench_options, test_cell};
use super::*;

#[test]
fn a_run_at_what_the_registry_declares_retraces_the_anchor() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    assert!(!opts.retargets_trajectory());
    opts.prescan = true;
    opts.checkpoint_override = Some(opts.plan.checkpoint);
    assert!(
        !opts.retargets_trajectory(),
        "diagnostics and a restated checkpoint move nothing"
    );
}

#[test]
fn the_cap_is_a_ceiling_not_a_retarget() {
    let cell = test_cell();
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, Some(&cell), &[]);
    opts.max_steps = 50_000;
    assert!(!opts.retargets_trajectory());
    assert!(
        incomparable_reasons(&opts)
            .iter()
            .any(|r| r.contains("--max-steps")),
        "the gate still refuses to compare a shortened run, and names the cap"
    );
}

#[test]
fn every_trajectory_override_the_gate_names_retargets_the_finish_line() {
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
        assert!(opts.retargets_trajectory(), "{label}");
        assert!(
            incomparable_reasons(&opts)
                .iter()
                .any(|r| r.contains(label)),
            "{label}: the gate must name the same override"
        );
    }
}
