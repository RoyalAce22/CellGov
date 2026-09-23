use super::entry::{run, run_inner};
use super::outcome::{
    comparison_exit_code, render_comparison, render_evaluation_summary, EvaluationOutcome,
    EXIT_HARNESS_FAILURE, EXIT_REGRESSION,
};

use std::path::{Path, PathBuf};

use cellgov_fuzz::evaluation::{
    compare, ComparisonVerdict, EvaluationResults, Metric, TrialOutcome, EVALUATION_SCHEMA_VERSION,
};
use cellgov_fuzz::{FuzzTarget, GenerationStrategy};

use crate::cli::exit::CommandExitCode;
use crate::cli::exit_codes;
use crate::cli::parse::{
    try_parse, Command, DevCommand, FuzzArgs, FuzzCommand, FuzzEvaluateEngine, FuzzStrategy,
    COMPARE_EXIT_CODES, EVALUATE_EXIT_CODES,
};

fn parse(argv: &[&str]) -> Result<FuzzArgs, clap::Error> {
    let mut args = vec!["cellgov", "dev", "fuzz"];
    args.extend_from_slice(argv);
    let cli = try_parse(&args.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    let Command::Dev(DevCommand::Fuzz(fuzz)) = cli.command else {
        panic!("fuzz must remain a dev command");
    };
    Ok(fuzz)
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("scratch paths are UTF-8").to_owned()
}

fn evaluate(engine: &str, output: &Path, extra: &[&str]) -> CommandExitCode {
    let output = path_arg(output);
    let mut args = vec![
        "evaluate",
        engine,
        "--trials",
        "4",
        "--cases",
        "12",
        "--workers",
        "2",
        "--output",
        &output,
    ];
    args.extend_from_slice(extra);
    let parsed = parse(&args).expect("evaluation parses");
    run_inner(&parsed).expect("evaluation runs")
}

fn read(path: &Path) -> EvaluationResults {
    EvaluationResults::parse_json(&std::fs::read_to_string(path).expect("results written"))
        .expect("stored results validate")
}

fn write(path: &Path, results: &EvaluationResults) {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(results).expect("serializes"),
    )
    .expect("written");
}

#[test]
fn evaluate_and_compare_parse_their_typed_settings() {
    let parsed = parse(&[
        "evaluate",
        "spu-sequence",
        "--trials",
        "3",
        "--first-seed",
        "40",
        "--cases",
        "8",
        "--sequence-words",
        "5",
        "--strategy",
        "raw-words",
        "--finding-limit",
        "9",
        "--reduction",
        "on-finding",
        "--reduction-budget",
        "64",
        "--workers",
        "1",
        "--progress",
        "--output",
        "results.json",
        "--baseline",
        "baseline.json",
    ])
    .expect("evaluate parses");
    let FuzzCommand::Evaluate(args) = parsed.command else {
        panic!("evaluate must select the evaluation runner");
    };
    assert!(matches!(args.engine, FuzzEvaluateEngine::SpuSequence));
    assert_eq!(
        (
            args.trials,
            args.first_seed,
            args.cases,
            args.sequence_words
        ),
        (3, 40, 8, Some(5))
    );
    assert!(matches!(args.strategy, FuzzStrategy::RawWords));
    assert_eq!((args.finding_limit, args.reduction_budget), (9, 64));
    assert_eq!((args.workers, args.progress), (Some(1), true));
    assert_eq!(args.output, PathBuf::from("results.json"));
    assert_eq!(args.baseline, Some(PathBuf::from("baseline.json")));

    let defaults =
        parse(&["evaluate", "ppu-instruction", "--output", "r.json"]).expect("defaults parse");
    let FuzzCommand::Evaluate(args) = defaults.command else {
        panic!("evaluate must select the evaluation runner");
    };
    assert_eq!((args.trials, args.first_seed, args.cases), (10, 1, 100));
    assert!(parse(&["evaluate", "ppu-instruction"]).is_err());

    let compared = parse(&["compare", "--baseline", "a.json", "--candidate", "b.json"])
        .expect("compare parses");
    let FuzzCommand::Compare(args) = compared.command else {
        panic!("compare must select the comparison runner");
    };
    assert_eq!(args.baseline, PathBuf::from("a.json"));
    assert_eq!(args.candidate, PathBuf::from("b.json"));
    assert!(parse(&["compare", "--baseline", "a.json"]).is_err());
}

#[test]
fn an_evaluation_writes_every_timed_trial_and_replays_from_the_file_alone() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_evaluate_writes");
    let output = scratch.join("nested").join("ppu.json");
    assert_eq!(
        evaluate("ppu-instruction", &output, &[]),
        CommandExitCode::SUCCESS
    );
    let results = read(&output);
    assert_eq!(results.schema_version, EVALUATION_SCHEMA_VERSION);
    assert_eq!(results.plan.target, FuzzTarget::PpuInstruction);
    assert_eq!(results.plan.strategy, GenerationStrategy::Structured);
    assert_eq!(results.plan.seeds, [1, 2, 3, 4]);
    assert_eq!(results.plan.budget.cases, 12);
    assert_eq!(results.environment.workers, 2);
    assert_eq!(results.environment.os, std::env::consts::OS);
    assert!(!results.environment.crate_version.is_empty());
    assert_eq!(results.trials.len(), 4);
    assert!(results
        .trials
        .iter()
        .all(|trial| trial.wall_ms.is_some() && trial.outcome == TrialOutcome::Clean));
    assert!(results.summary.distributions.contains_key(&Metric::WallMs));

    // The stored plan reproduces every count without the file's trials.
    let replayed = results
        .plan
        .seeds
        .iter()
        .map(|seed| cellgov_fuzz::evaluation::run_trial(&results.plan, *seed).expect("replays"))
        .collect::<Vec<_>>();
    for (stored, again) in results.trials.iter().zip(&replayed) {
        assert_eq!(stored.counts, again.counts);
        assert_eq!(stored.findings, again.findings);
    }

    let text = render_evaluation_summary(&EvaluationOutcome {
        target: results.plan.target,
        output: output.clone(),
        summary: results.summary.clone(),
    });
    assert!(text.starts_with("fuzz evaluate: PpuInstruction trials=4 cases_per_trial=12 output="));
    assert!(text.contains("fuzz evaluate: Eligible samples=4 min="));
    assert!(text.contains("fuzz evaluate: WallMs samples=4"));
}

#[test]
fn a_candidate_that_matches_its_baseline_is_indistinguishable_and_succeeds() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_evaluate_baseline");
    let baseline = scratch.join("baseline.json");
    let candidate = scratch.join("candidate.json");
    assert_eq!(
        evaluate("spu-instruction", &baseline, &[]),
        CommandExitCode::SUCCESS
    );
    assert_eq!(
        evaluate(
            "spu-instruction",
            &candidate,
            &["--baseline", &path_arg(&baseline)]
        ),
        CommandExitCode::SUCCESS
    );
    let parsed = parse(&[
        "compare",
        "--baseline",
        &path_arg(&baseline),
        "--candidate",
        &path_arg(&candidate),
    ])
    .expect("compare parses");
    assert_eq!(
        run_inner(&parsed).expect("comparison runs"),
        CommandExitCode::SUCCESS
    );
    let comparison = compare(&read(&baseline), &read(&candidate)).expect("comparable");
    assert_eq!(comparison.verdict(), ComparisonVerdict::Indistinguishable);
    let text = render_comparison(&comparison);
    assert!(text.starts_with(
        "fuzz compare: trials=4 cases_per_trial=12 verdict=Indistinguishable regressions=[]"
    ));
    assert!(text.contains("fuzz compare: Eligible baseline_median="));
}

#[test]
fn a_regressed_candidate_exits_with_the_regression_status() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_evaluate_regression");
    let baseline = scratch.join("baseline.json");
    assert_eq!(
        evaluate("ppu-instruction", &baseline, &[]),
        CommandExitCode::SUCCESS
    );
    let stored = read(&baseline);
    // A candidate whose generator reached nothing: same plan, every count zero.
    let mut trials = stored.trials.clone();
    for trial in &mut trials {
        trial.counts.eligible = 0;
        trial.counts.executed_steps = 0;
        trial.counts.instruction_kinds = 0;
        trial.counts.effect_classes = 0;
        trial.counts.metamorphic_executions = 0;
        trial.outcome = TrialOutcome::NoEligibleCases;
    }
    let regressed =
        EvaluationResults::from_trials(stored.plan.clone(), stored.environment.clone(), trials)
            .expect("complete");
    let candidate = scratch.join("candidate.json");
    write(&candidate, &regressed);

    let parsed = parse(&[
        "compare",
        "--baseline",
        &path_arg(&baseline),
        "--candidate",
        &path_arg(&candidate),
    ])
    .expect("compare parses");
    assert_eq!(
        run_inner(&parsed).expect("comparison runs"),
        CommandExitCode::new(EXIT_REGRESSION)
    );
    let reversed = parse(&[
        "compare",
        "--baseline",
        &path_arg(&candidate),
        "--candidate",
        &path_arg(&baseline),
    ])
    .expect("compare parses");
    assert_eq!(
        run_inner(&reversed).expect("comparison runs"),
        CommandExitCode::SUCCESS
    );
    assert_eq!(
        comparison_exit_code(ComparisonVerdict::Regressed),
        CommandExitCode::new(EXIT_REGRESSION)
    );
    assert_eq!(
        comparison_exit_code(ComparisonVerdict::Improved),
        CommandExitCode::SUCCESS
    );
    assert_eq!(
        comparison_exit_code(ComparisonVerdict::Indistinguishable),
        CommandExitCode::SUCCESS
    );
}

#[test]
fn unrunnable_plans_and_unrankable_results_are_usage_refusals() {
    let scratch = cellgov_testkit::scratch::scratch_labeled("fuzz_evaluate_refusals");
    let output = path_arg(&scratch.join("never.json"));
    for args in [
        vec![
            "evaluate",
            "ppu-instruction",
            "--trials",
            "1",
            "--output",
            &output,
        ],
        vec![
            "evaluate",
            "ppu-instruction",
            "--cases",
            "0",
            "--output",
            &output,
        ],
        vec![
            "evaluate",
            "ppu-instruction",
            "--sequence-words",
            "4",
            "--output",
            &output,
        ],
        vec![
            "evaluate",
            "spu-sequence",
            "--sequence-words",
            "0",
            "--output",
            &output,
        ],
        vec![
            "evaluate",
            "ppu-instruction",
            "--finding-limit",
            "0",
            "--output",
            &output,
        ],
        vec![
            "evaluate",
            "ppu-instruction",
            "--reduction",
            "on-finding",
            "--reduction-budget",
            "0",
            "--output",
            &output,
        ],
    ] {
        let parsed = parse(&args).expect("typed flags parse");
        let error = run(&parsed).expect_err("the plan is refused");
        assert_eq!(
            error.code().expect("status").value(),
            exit_codes::USAGE as u8,
            "{args:?}"
        );
    }
    assert!(!scratch.join("never.json").exists());

    let baseline = scratch.join("baseline.json");
    assert_eq!(
        evaluate("spu-instruction", &baseline, &[]),
        CommandExitCode::SUCCESS
    );
    let wider = scratch.join("wider.json");
    assert_eq!(
        evaluate(
            "spu-instruction",
            &wider,
            &["--baseline", &path_arg(&baseline)]
        ),
        CommandExitCode::SUCCESS
    );
    let other = scratch.join("other.json");
    let parsed = parse(&[
        "evaluate",
        "spu-instruction",
        "--trials",
        "3",
        "--cases",
        "12",
        "--workers",
        "1",
        "--output",
        &path_arg(&other),
        "--baseline",
        &path_arg(&baseline),
    ])
    .expect("evaluate parses");
    let error = run(&parsed).expect_err("a different trial count cannot be ranked");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::USAGE as u8
    );
    assert!(
        !other.exists(),
        "a baseline that cannot rank the plan is refused before any trial runs"
    );
    let engine = scratch.join("engine.json");
    let parsed = parse(&[
        "evaluate",
        "ppu-instruction",
        "--trials",
        "4",
        "--cases",
        "12",
        "--workers",
        "1",
        "--output",
        &path_arg(&engine),
        "--baseline",
        &path_arg(&baseline),
    ])
    .expect("evaluate parses");
    let error = run(&parsed).expect_err("a different engine cannot be ranked");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::USAGE as u8
    );
    assert!(
        !engine.exists(),
        "the engine mismatch is refused before any trial runs"
    );
    let unread = scratch.join("unread.json");
    let parsed = parse(&[
        "evaluate",
        "spu-instruction",
        "--trials",
        "4",
        "--cases",
        "12",
        "--workers",
        "1",
        "--output",
        &path_arg(&unread),
        "--baseline",
        &path_arg(&scratch.join("absent.json")),
    ])
    .expect("evaluate parses");
    let error = run(&parsed).expect_err("an unreadable baseline fails");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::FAILED as u8
    );
    assert!(
        !unread.exists(),
        "an unreadable baseline is refused before any trial runs"
    );

    let missing = parse(&[
        "compare",
        "--baseline",
        &path_arg(&scratch.join("absent.json")),
        "--candidate",
        &path_arg(&baseline),
    ])
    .expect("compare parses");
    let error = run(&missing).expect_err("an unreadable result fails");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::FAILED as u8
    );

    let mut incomplete = read(&baseline);
    incomplete.trials.pop();
    let truncated = scratch.join("truncated.json");
    write(&truncated, &incomplete);
    let parsed = parse(&[
        "compare",
        "--baseline",
        &path_arg(&baseline),
        "--candidate",
        &path_arg(&truncated),
    ])
    .expect("compare parses");
    let error = run(&parsed).expect_err("a truncated result fails");
    assert_eq!(
        error.code().expect("status").value(),
        exit_codes::FAILED as u8
    );
}

#[test]
fn the_help_names_every_evaluation_status() {
    for (help, codes) in [
        (
            EVALUATE_EXIT_CODES,
            vec![exit_codes::FAILED, EXIT_HARNESS_FAILURE, EXIT_REGRESSION],
        ),
        (
            COMPARE_EXIT_CODES,
            vec![exit_codes::FAILED, EXIT_REGRESSION],
        ),
    ] {
        for code in codes {
            assert!(
                help.lines().any(|line| {
                    line.split_whitespace()
                        .next()
                        .is_some_and(|first| first == code.to_string())
                }),
                "help does not name {code}:\n{help}"
            );
        }
    }
}
