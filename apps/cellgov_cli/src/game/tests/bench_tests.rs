//! Bench-result line parsing, wall-clock disagreement math, and the
//! anchor comparison the pair gate runs.

use super::*;

#[test]
fn wall_disagreement_percent_is_zero_for_identical_durations() {
    use std::time::Duration;
    assert_eq!(
        wall_disagreement_percent(Duration::from_millis(1000), Duration::from_millis(1000)),
        Some(0.0)
    );
}

#[test]
fn wall_disagreement_percent_is_relative_to_faster_run() {
    use std::time::Duration;
    let pct = wall_disagreement_percent(Duration::from_millis(100), Duration::from_millis(105))
        .expect("finite");
    assert!((pct - 5.0).abs() < 0.0001, "expected 5.0, got {pct}");
}

#[test]
fn wall_disagreement_percent_is_symmetric() {
    use std::time::Duration;
    let a = wall_disagreement_percent(Duration::from_millis(200), Duration::from_millis(250));
    let b = wall_disagreement_percent(Duration::from_millis(250), Duration::from_millis(200));
    assert_eq!(a, b);
}

#[test]
fn wall_disagreement_percent_returns_none_on_zero_duration() {
    use std::time::Duration;
    assert_eq!(
        wall_disagreement_percent(Duration::ZERO, Duration::from_millis(100)),
        None
    );
    assert_eq!(
        wall_disagreement_percent(Duration::from_millis(100), Duration::ZERO),
        None
    );
    assert_eq!(
        wall_disagreement_percent(Duration::ZERO, Duration::ZERO),
        None
    );
}

#[test]
fn parse_bench_result_round_trips_every_boot_outcome() {
    let variants = [
        BootOutcome::ProcessExit,
        BootOutcome::Fault,
        BootOutcome::MaxSteps,
        BootOutcome::RsxWriteCheckpoint,
        BootOutcome::PcReached(0x10381ce8),
        BootOutcome::TimeOverflow,
    ];
    for v in variants {
        let line = format!("BENCH_RESULT steps=1 wall_us=1 steps_per_sec=1000000 outcome={v}\n");
        let r = parse_bench_result(&line)
            .unwrap_or_else(|e| panic!("round-trip parse failed for {v:?}: {e}"));
        assert_eq!(r.outcome, v, "round-trip mismatch for {v:?}");
    }
}

#[test]
fn parse_bench_result_extracts_fields() {
    let stdout = "some preamble\nBENCH_RESULT steps=1402388 wall_us=323000 steps_per_sec=4342377 outcome=ProcessExit\ntrailing noise\n";
    let r = parse_bench_result(stdout).expect("parses");
    assert_eq!(r.steps, 1402388);
    assert_eq!(r.wall.as_millis(), 323);
    assert_eq!(r.outcome, BootOutcome::ProcessExit);
}

#[test]
fn parse_bench_result_errors_on_missing_line() {
    let stdout = "just some noise\nbut no result line\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::NoResultLine
    );
}

#[test]
fn parse_bench_result_errors_on_duplicate_line() {
    let stdout = "BENCH_RESULT steps=1 wall_us=1 steps_per_sec=1 outcome=ProcessExit\n\
                  BENCH_RESULT steps=2 wall_us=2 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::DuplicateResultLine
    );
}

#[test]
fn parse_bench_result_errors_on_unknown_outcome() {
    let stdout = "BENCH_RESULT steps=1 wall_us=1 steps_per_sec=1 outcome=WhoKnows\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::UnparseableOutcome { token, source: _ } => {
            assert_eq!(token, "WhoKnows");
        }
        other => panic!("expected UnparseableOutcome, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_malformed_steps() {
    let stdout = "BENCH_RESULT steps=abc wall_us=1 steps_per_sec=1 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedSteps(s) => assert_eq!(s, "abc"),
        other => panic!("expected MalformedSteps, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_missing_steps() {
    let stdout = "BENCH_RESULT wall_us=1 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingSteps
    );
}

#[test]
fn parse_bench_result_errors_on_malformed_wall_us() {
    let stdout = "BENCH_RESULT steps=1 wall_us=xyz steps_per_sec=1 outcome=ProcessExit\n";
    match parse_bench_result(stdout).unwrap_err() {
        ParseBenchError::MalformedWallUs(s) => assert_eq!(s, "xyz"),
        other => panic!("expected MalformedWallUs, got {other:?}"),
    }
}

#[test]
fn parse_bench_result_errors_on_missing_wall_us() {
    let stdout = "BENCH_RESULT steps=1 steps_per_sec=1 outcome=ProcessExit\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingWallUs
    );
}

#[test]
fn parse_bench_result_errors_on_missing_outcome() {
    let stdout = "BENCH_RESULT steps=1 wall_us=1 steps_per_sec=1\n";
    assert_eq!(
        parse_bench_result(stdout).unwrap_err(),
        ParseBenchError::MissingOutcome
    );
}

#[test]
fn classify_pair_pass() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(102),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(
        classify_pair(&r1, &r2, drift, &[], &AnchorVerdict::Skipped),
        BenchGate::Pass
    );
}

#[test]
fn classify_pair_determinism_break_on_step_mismatch() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 11,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(
        classify_pair(&r1, &r2, drift, &[], &AnchorVerdict::Skipped),
        BenchGate::DeterminismBreak
    );
}

#[test]
fn classify_pair_determinism_break_on_outcome_mismatch() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::MaxSteps,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(
        classify_pair(&r1, &r2, drift, &[], &AnchorVerdict::Skipped),
        BenchGate::DeterminismBreak
    );
}

#[test]
fn classify_pair_wall_drift_exceeded() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(200),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    assert_eq!(
        classify_pair(&r1, &r2, drift, &[], &AnchorVerdict::Skipped),
        BenchGate::WallDriftExceeded
    );
}

#[test]
fn classify_pair_wall_unmeasurable() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::ZERO,
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    assert_eq!(
        classify_pair(&r1, &r2, None, &[], &AnchorVerdict::Skipped),
        BenchGate::WallUnmeasurable
    );
}

/// Mirrors a committed `boot_summary.json`, so the fixture format and
/// the comparison are exercised through the deserializer production
/// uses.
fn anchor_fixture(breaks: u64) -> BootSummary {
    serde_json::from_str(&format!(
        r#"{{
          "checkpoint": {{ "kind": "process_exit" }},
          "outcome": "MaxSteps",
          "steps": 390099,
          "budget": 256,
          "host_invariant_breaks": {breaks},
          "witnesses": {{
            "host_invariant_breaks": {{ "value": {breaks}, "class": "exact" }},
            "ldarx": {{ "value": 100, "class": "at-least" }},
            "stdcx": {{ "value": 0, "class": "at-least" }},
            "lwarx": {{ "value": 0, "class": "at-least" }},
            "stwcx": {{ "value": 0, "class": "at-least" }}
          }}
        }}"#
    ))
    .expect("anchor fixture parses")
}

fn observed_stderr(breaks: u64, ldarx: u64) -> ParsedWitnesses {
    parse_witness_lines(&format!(
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count={breaks}\n\
         BENCH_ATOMIC_WITNESS: ldarx={ldarx} stdcx=0 lwarx=0 stwcx=0\n"
    ))
    .expect("synthetic witness lines parse")
}

#[test]
fn a_run_matching_its_anchor_reports_no_disagreements() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        390099,
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
        390099,
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
        390099,
        "MaxSteps",
        &observed_stderr(73, 9_999),
    );
    assert!(failures.is_empty(), "got {failures:?}");
}

#[test]
fn a_moved_step_count_is_reported() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        390100,
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
        390099,
        "ProcessExit",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("outcome ProcessExit")),
        "got {failures:?}"
    );
}

/// `FromStr` round-trips the Display form, so the comparison must use
/// it too: Debug renders the address in decimal and would report a
/// mismatch against an identical outcome.
#[test]
fn a_pc_reached_outcome_compares_by_its_display_form() {
    let mut baseline = anchor_fixture(73);
    baseline.outcome = BootOutcome::PcReached(0x1_0000);
    let observed = observed_stderr(73, 100);
    let same = anchor_disagreements(&baseline, 390099, "PcReached(0x10000)", &observed);
    assert!(same.is_empty(), "identical outcome must match: {same:?}");
    let debug_form = anchor_disagreements(&baseline, 390099, "PcReached(65536)", &observed);
    assert!(
        !debug_form.is_empty(),
        "the decimal Debug form is not equal"
    );
}

#[test]
fn a_title_with_no_committed_anchor_is_skipped_not_failed() {
    let verdict = check_anchor(
        "CG_NO_SUCH_CONTENT_ID",
        1,
        "MaxSteps",
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n",
    );
    assert_eq!(verdict, AnchorVerdict::NoBaseline);
}

/// The workspace root is compiled in, so every title looks unrecorded
/// once the binary leaves its source tree. Saying so is the difference
/// between a reported skip and a gate that quietly stopped gating.
#[test]
fn an_unreachable_workspace_root_does_not_read_as_an_unrecorded_title() {
    let absent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("no_such_workspace_root");
    let verdict = check_anchor_under(
        &absent,
        "VSH",
        1,
        "MaxSteps",
        "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=0\n",
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
fn anchor_drift_outranks_wall_drift() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(200),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    let anchor = AnchorVerdict::Drift(vec!["host_invariant_breaks moved".to_string()]);
    assert_eq!(
        classify_pair(&r1, &r2, drift, &[], &anchor),
        BenchGate::AnchorDrift,
        "a contended host must not mask a real anchor regression",
    );
}

#[test]
fn a_determinism_break_outranks_anchor_drift() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let r2 = BenchBootResult {
        steps: 11,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r2.wall);
    let anchor = AnchorVerdict::Drift(vec!["host_invariant_breaks moved".to_string()]);
    assert_eq!(
        classify_pair(&r1, &r2, drift, &[], &anchor),
        BenchGate::DeterminismBreak,
    );
}

#[test]
fn a_skipped_anchor_check_cannot_produce_anchor_drift() {
    use std::time::Duration;
    let r1 = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(r1.wall, r1.wall);
    assert_eq!(
        classify_pair(&r1, &r1, drift, &[], &AnchorVerdict::Skipped),
        BenchGate::Pass
    );
    assert_eq!(
        classify_pair(&r1, &r1, drift, &[], &AnchorVerdict::NoBaseline),
        BenchGate::Pass
    );
    assert_eq!(
        classify_pair(
            &r1,
            &r1,
            drift,
            &[],
            &AnchorVerdict::NotComparable(vec!["retargeted".to_string()])
        ),
        BenchGate::Pass
    );
}

#[test]
fn an_anchor_with_no_witnesses_is_a_disagreement() {
    let mut baseline = anchor_fixture(73);
    baseline.witnesses.clear();
    let failures = anchor_disagreements(&baseline, 390099, "MaxSteps", &observed_stderr(73, 100));
    assert_eq!(failures, vec!["anchor records no witnesses".to_string()]);
}

#[test]
fn a_recorded_witness_whose_line_never_appeared_is_reported() {
    let observed = parse_witness_lines("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n")
        .expect("synthetic witness line parses");
    let failures = anchor_disagreements(&anchor_fixture(73), 390099, "MaxSteps", &observed);
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
    let failures = anchor_disagreements(&anchor_fixture(73), 390099, "MaxSteps", &observed);
    assert_eq!(
        failures,
        vec!["witness dcbz is emitted but not recorded in the anchor".to_string()]
    );
}

#[test]
fn a_zero_step_run_against_a_recorded_anchor_is_a_disagreement() {
    let failures = anchor_disagreements(
        &anchor_fixture(73),
        0,
        "MaxSteps",
        &observed_stderr(73, 100),
    );
    assert!(
        failures.iter().any(|f| f.contains("steps 0")),
        "got {failures:?}"
    );
}

fn bench_manifest(bench_max_steps: Option<u64>) -> crate::game::manifest::TitleManifest {
    use crate::game::manifest::{Distribution, GameSource};
    crate::game::manifest::TitleManifest {
        content_id: "CG_TEST".to_string(),
        short_name: "test".to_string(),
        display_name: "test".to_string(),
        eboot_candidates: vec!["EBOOT.BIN".to_string()],
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps,
        checkpoint: manifest::CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
    }
}

fn bench_options<'a>(
    title: &'a crate::game::manifest::TitleManifest,
    guest_args: &'a [String],
) -> BenchOptions<'a> {
    BenchOptions {
        title,
        elf_path: "EBOOT.BIN",
        max_steps: DEFAULT_BENCH_MAX_STEPS as usize,
        firmware_dir: None,
        strict_reserved: false,
        checkpoint_override: None,
        budget_override: None,
        prescan: false,
        guest_args,
        check_anchor: true,
    }
}

#[test]
fn a_run_at_the_recorded_cap_is_comparable() {
    let title = bench_manifest(None);
    assert!(incomparable_reasons(&bench_options(&title, &[])).is_empty());
    let capped = bench_manifest(Some(4_000));
    let mut opts = bench_options(&capped, &[]);
    opts.max_steps = 4_000;
    assert!(incomparable_reasons(&opts).is_empty());
}

/// A diagnostic-only flag must not disable the gate: `--prescan` only
/// prints a decode report before execution.
#[test]
fn prescan_leaves_the_run_comparable() {
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, &[]);
    opts.prescan = true;
    assert!(incomparable_reasons(&opts).is_empty());
}

#[test]
fn a_shortened_run_is_not_compared_against_the_anchor() {
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, &[]);
    opts.max_steps = 50_000;
    let reasons = incomparable_reasons(&opts);
    assert_eq!(reasons.len(), 1, "got {reasons:?}");
    assert!(reasons[0].contains("--max-steps 50000"), "got {reasons:?}");
}

#[test]
fn every_trajectory_override_names_itself_as_incomparable() {
    let title = bench_manifest(None);
    let args = vec!["EBOOT.BIN".to_string()];

    let mut checkpoint = bench_options(&title, &[]);
    checkpoint.checkpoint_override = Some(manifest::CheckpointTrigger::Pc(0x1_0000));
    let mut budget = bench_options(&title, &[]);
    budget.budget_override = Some(Budget::new(512));
    let mut strict = bench_options(&title, &[]);
    strict.strict_reserved = true;
    let guest = bench_options(&title, &args);

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

/// Restating the manifest's own checkpoint is not a retarget, so it
/// must not disable the comparison.
#[test]
fn a_checkpoint_override_equal_to_the_manifest_stays_comparable() {
    let title = bench_manifest(None);
    let mut opts = bench_options(&title, &[]);
    opts.checkpoint_override = Some(title.checkpoint_trigger());
    assert!(incomparable_reasons(&opts).is_empty());
}

#[test]
fn identical_witness_streams_disagree_nowhere() {
    let stream = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n\
                  BENCH_ATOMIC_WITNESS: ldarx=100 stdcx=0 lwarx=0 stwcx=0\n";
    assert!(witness_disagreements(stream, stream).is_empty());
}

/// The steps/outcome comparison cannot see this, and the anchor check
/// reads run 1 alone, so without the pairwise witness check a counter
/// that moves between runs passes the gate.
#[test]
fn a_witness_that_moved_between_runs_is_a_determinism_break() {
    let r1 = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n";
    let r2 = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=76\n";
    let failures = witness_disagreements(r1, r2);
    assert_eq!(
        failures,
        vec!["witness host_invariant_breaks: run 1 73 != run 2 76".to_string()]
    );

    use std::time::Duration;
    let run = BenchBootResult {
        steps: 10,
        wall: Duration::from_millis(100),
        outcome: BootOutcome::ProcessExit,
    };
    let drift = wall_disagreement_percent(run.wall, run.wall);
    assert_eq!(
        classify_pair(&run, &run, drift, &failures, &AnchorVerdict::Match),
        BenchGate::DeterminismBreak,
        "agreeing steps, outcome and anchor must not outvote a moving witness",
    );
}

#[test]
fn a_witness_line_only_one_run_emitted_is_a_disagreement() {
    let r1 = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n\
              BENCH_DCBZ_WITNESS: count=0\n";
    let r2 = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n";
    let failures = witness_disagreements(r1, r2);
    assert_eq!(
        failures,
        vec![
            "witness line BENCH_DCBZ_WITNESS: appeared in run 1 only".to_string(),
            "witness dcbz: run 1 0, absent from run 2".to_string(),
        ]
    );
}

#[test]
fn a_malformed_witness_line_in_either_run_is_a_disagreement() {
    let good = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=73\n";
    let bad = "BENCH_HOST_INVARIANT_BREAKS_WITNESS: count=lots\n";
    assert!(witness_disagreements(good, bad)[0].starts_with("run 2 witness line did not parse"));
    assert!(witness_disagreements(bad, good)[0].starts_with("run 1 witness line did not parse"));
}
