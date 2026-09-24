//! Repeated equal-budget trials and their ranking against a stored baseline.

use std::path::{Path, PathBuf};
use std::time::Instant;

use cellgov_fuzz::evaluation::{
    comparable, compare, run_trial, EvaluationEnvironment, EvaluationPlan, EvaluationResults,
    TrialOutcome, TrialRecord, CRATE_VERSION,
};
use cellgov_fuzz::reduce::ReductionRequest;
use cellgov_fuzz::FuzzTarget;

use super::campaign::{check_finding_limit, run_workers};
use super::entry::{reports_progress, worker_count, write_stdout};
use super::error::FuzzCliError;
use super::outcome::{
    comparison_exit_code, render_comparison, render_evaluation_progress, render_evaluation_summary,
    EvaluationOutcome,
};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::{FuzzCompareArgs, FuzzEvaluateArgs, FuzzReduction};

/// Runs one evaluation as a fixed sequence of stages.
///
/// Every refusal of the request comes before the first trial runs.
/// [Manes2021 p:3 s:2.3 Fuzz Testing Algorithm] A model fuzzer is a fixed sequence of
/// stages with separate design decisions.
pub(super) fn run_evaluate(
    args: &FuzzEvaluateArgs,
    quiet: bool,
) -> Result<CommandExitCode, FuzzCliError> {
    let plan = plan(args)?;
    let workers = worker_count(args.workers)?;
    let workers_u32 =
        u32::try_from(workers).map_err(|_| FuzzCliError::Invalid("workers exceed u32"))?;
    let baseline = args
        .baseline
        .as_ref()
        .map(|path| -> Result<EvaluationResults, FuzzCliError> {
            let baseline = read_results(path)?;
            comparable(&baseline.plan, &plan)?;
            Ok(baseline)
        })
        .transpose()?;
    let trials = run_trials(&plan, workers, args.progress, quiet)?;
    let environment = EvaluationEnvironment {
        crate_version: CRATE_VERSION.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        workers: workers_u32,
    };
    let harness_failure = trials.iter().find_map(|trial| match &trial.outcome {
        TrialOutcome::HarnessFailure { message } => Some((trial.seed, message.clone())),
        _ => None,
    });
    let results = EvaluationResults::from_trials(plan, environment, trials)?;
    write_results(&args.output, &results)?;
    write_stdout(&render_evaluation_summary(&EvaluationOutcome {
        target: results.plan.target,
        output: args.output.clone(),
        summary: results.summary.clone(),
    }))?;
    if let Some((seed, message)) = harness_failure {
        return Err(FuzzCliError::TrialHarness { seed, message });
    }
    match baseline {
        Some(baseline) => {
            let comparison = compare(&baseline, &results)?;
            write_stdout(&render_comparison(&comparison))?;
            Ok(comparison_exit_code(comparison.verdict()))
        }
        None => Ok(CommandExitCode::SUCCESS),
    }
}

pub(super) fn run_compare(args: &FuzzCompareArgs) -> Result<CommandExitCode, FuzzCliError> {
    let baseline = read_results(&args.baseline)?;
    let candidate = read_results(&args.candidate)?;
    let comparison = compare(&baseline, &candidate)?;
    write_stdout(&render_comparison(&comparison))?;
    Ok(comparison_exit_code(comparison.verdict()))
}

fn plan(args: &FuzzEvaluateArgs) -> Result<EvaluationPlan, FuzzCliError> {
    let target = FuzzTarget::from(args.engine);
    if !target.generates_sequences() && args.sequence_words.is_some() {
        return Err(FuzzCliError::Invalid(
            "sequence-words applies only to sequence engines",
        ));
    }
    check_finding_limit(args.finding_limit)?;
    let reduction = match args.reduction {
        FuzzReduction::None => None,
        FuzzReduction::OnFinding => Some(ReductionRequest {
            policy: args.reduction_policy.into(),
            budget: args.reduction_budget,
        }),
    };
    if reduction.is_some_and(|request| request.budget == 0) {
        return Err(FuzzCliError::Invalid("reduction-budget must be positive"));
    }
    if args.output.to_str().is_none() {
        return Err(FuzzCliError::Invalid("output must be valid UTF-8"));
    }
    let plan = EvaluationPlan {
        sequence_words: args
            .sequence_words
            .unwrap_or(cellgov_fuzz::DEFAULT_SEQUENCE_WORDS),
        max_findings: args.finding_limit,
        reduction,
        ..EvaluationPlan::new(
            target,
            args.strategy.into(),
            args.cases,
            args.first_seed,
            args.trials,
        )
    };
    plan.validate()?;
    Ok(plan)
}

/// Runs every planned trial, `workers` at a time, and times each one.
fn run_trials(
    plan: &EvaluationPlan,
    workers: usize,
    progress: bool,
    quiet: bool,
) -> Result<Vec<TrialRecord>, FuzzCliError> {
    let mut trials = Vec::with_capacity(plan.seeds.len());
    for batch in plan.seeds.chunks(workers.max(1)) {
        let work = batch
            .iter()
            .map(|&seed| {
                move || {
                    let start = Instant::now();
                    let trial = run_trial(plan, seed);
                    let wall_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                    trial.map(|mut trial| {
                        trial.wall_ms = Some(wall_ms);
                        trial
                    })
                }
            })
            .collect::<Vec<_>>();
        for trial in run_workers(work)? {
            let trial = trial?;
            if reports_progress(progress, quiet) {
                eprintln!(
                    "{}",
                    render_evaluation_progress(
                        trial.seed,
                        trials.len() as u64 + 1,
                        plan.seeds.len() as u64
                    )
                );
            }
            trials.push(trial);
        }
    }
    Ok(trials)
}

fn read_results(path: &Path) -> Result<EvaluationResults, FuzzCliError> {
    let json = std::fs::read_to_string(path).map_err(|source| FuzzCliError::EvaluationRead {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(EvaluationResults::parse_json(&json)?)
}

fn write_results(path: &PathBuf, results: &EvaluationResults) -> Result<(), FuzzCliError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| FuzzCliError::Write {
            path: path.clone(),
            source,
        })?;
    }
    let encoded = serde_json::to_vec_pretty(results)?;
    std::fs::write(path, encoded).map_err(|source| FuzzCliError::Write {
        path: path.clone(),
        source,
    })
}
