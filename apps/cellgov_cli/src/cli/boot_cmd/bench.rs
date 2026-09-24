//! `boot bench-once` and `boot bench`: one measurement, a run set, and the verdict rendering.

use std::path::Path;

use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressBar;
use cellgov_time::Budget;

use crate::game;
use crate::progress::{BENCH_PAIR_TASK, BENCH_TASK};
use cellgov_boot::compose::ResolvedPlan;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::{BenchArgs, BenchGateArgs};
use crate::cli::title::resolve_ps3_vfs_root;

use super::compose::{anchor_plan, firmware_module_dir, plan_max_steps, selection_args};
use super::inputs::resolve_boot_inputs;

/// Exit code: the runs of a set disagreed on step count, outcome or a
/// witness.
const EXIT_DETERMINISM_BREAK: i32 = exit_codes::DISAGREED;

/// Exit code: `--strict-perf` is set and the run set reaches no
/// throughput verdict.
pub(in crate::cli) const EXIT_SPREAD_EXCEEDED: i32 = exit_codes::command_specific(15);

/// Exit code: a bench subprocess failed or its `BENCH_RESULT` line was
/// unparseable.
const EXIT_SUBPROCESS_FAIL: i32 = exit_codes::DIVERGED;

/// Exit code: the run disagreed with the title's committed anchor.
const EXIT_ANCHOR_DRIFT: i32 = exit_codes::ANCHOR_MOVED;

pub(in crate::cli) fn separate_spawn_command_error(
    error: game::SpawnError,
) -> Result<game::SpawnError, CommandError> {
    match error {
        game::SpawnError::Command(error) => Err(error),
        error => Ok(error),
    }
}

pub(crate) fn bench_boot_once(
    args: &BenchArgs,
    vfs_flag: Option<&Path>,
    render: RenderFlags,
) -> Result<CommandExitCode, CommandError> {
    let vfs_root = resolve_ps3_vfs_root(vfs_flag)?;
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        args.overrides.overrides(),
        &vfs_root,
        None,
        "boot bench-once",
    )?;
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    let max_steps = match args.max_steps {
        Some(max_steps) => max_steps,
        None => plan_max_steps(&plan, &inputs.title)?,
    };
    let firmware_dir = firmware_module_dir(&inputs.composition)?;
    let selection = selection_args(&args.selection, vfs_flag)?;
    let bar = ProgressBar::start(render.caps(), &BENCH_TASK, inputs.title.name());
    let sink = bar.sink();
    let result = game::bench_boot_one_run(
        game::BenchOptions {
            title: &inputs.title,
            elf_path: &inputs.elf_path,
            max_steps,
            plan: anchor_plan(&plan),
            firmware_dir: firmware_dir.as_deref(),
            composed_mounts: &inputs.composition.mounts,
            eboot_dirs: &inputs.composition.eboot_dirs,
            identity: &inputs.composition.identity,
            selection: selection.as_args(),
            strict_reserved: args.strict_reserved,
            checkpoint_override: args.checkpoint,
            budget_override: args.budget.map(Budget::new),
            prescan: args.prescan,
            guest_args: &args.guest_arg,
            // This entry point is the raw measurement a run set spawns
            // once per run; only the set gates on the anchor.
            check_anchor: false,
            run_index: args.run_index.unwrap_or(0),
        },
        inputs.elf_data,
        inputs.authority_id,
        inputs.control_flags1,
        args.save_state_trace.as_deref(),
        &*sink,
    );
    match result {
        Ok(_) => bar.finish(),
        Err(error) => {
            bar.abort();
            return Err(CommandError::failed(error.to_string()));
        }
    }
    Ok(CommandExitCode::SUCCESS)
}

/// Both `bench` and `bench-once` flatten [`BenchArgs`], so clap accepts
/// `--save-state-trace` and `--run-index` on either. The run set
/// forwards neither to its children.
fn refuse_bench_once_only_flags(args: &BenchArgs) -> Result<(), CommandError> {
    if let Some(path) = &args.save_state_trace {
        return Err(CommandError::status(
            exit_codes::USAGE,
            format!(
                "boot bench: --save-state-trace {path} names one path, and a run set takes \
             several measurements that would each write over it. A traced boot is a \
             divergence diagnostic rather than a measurement, so take it with \
             `boot bench-once --save-state-trace PATH`."
            ),
        ));
    }
    if let Some(index) = args.run_index {
        return Err(CommandError::status(
            exit_codes::USAGE,
            format!(
                "boot bench: --run-index {index} has no meaning for a run set: the set stamps \
             each child it spawns with that child's own index. Pass it to \
             `boot bench-once` only."
            ),
        ));
    }
    Ok(())
}

/// One measurement spreads against nothing, so a strict gate over it
/// would report OK for a check that never runs.
fn refuse_strict_perf_without_a_spread(gate_args: &BenchGateArgs) -> Result<(), CommandError> {
    if gate_args.strict_perf && gate_args.runs < 2 {
        return Err(CommandError::status(
            exit_codes::USAGE,
            "boot bench: --strict-perf enforces the cross-run spread, and --runs 1 \
             measures no spread to enforce. Take at least two runs, or drop \
             --strict-perf.",
        ));
    }
    Ok(())
}

pub(crate) fn bench_boot(
    gate_args: &BenchGateArgs,
    vfs_flag: Option<&Path>,
    render: RenderFlags,
) -> Result<CommandExitCode, CommandError> {
    let args: &BenchArgs = &gate_args.bench;
    // Ahead of every resolution below: a refused invocation must not
    // first read the store.
    refuse_bench_once_only_flags(args)?;
    refuse_strict_perf_without_a_spread(gate_args)?;
    if gate_args.all {
        return crate::cli::bench_all::run(gate_args, vfs_flag, render);
    }
    let vfs_root = resolve_ps3_vfs_root(vfs_flag)?;
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        args.overrides.overrides(),
        &vfs_root,
        None,
        "boot bench",
    )?;
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    let max_steps = match args.max_steps {
        Some(max_steps) => max_steps,
        None => plan_max_steps(&plan, &inputs.title)?,
    };
    let firmware_dir = firmware_module_dir(&inputs.composition)?;
    let selection = selection_args(&args.selection, vfs_flag)?;
    let bar = ProgressBar::start(render.caps(), &BENCH_PAIR_TASK, inputs.title.name());
    let sink = bar.sink();
    let outcome = match game::bench_boot_runs(
        game::BenchOptions {
            title: &inputs.title,
            elf_path: &inputs.elf_path,
            max_steps,
            plan: anchor_plan(&plan),
            firmware_dir: firmware_dir.as_deref(),
            composed_mounts: &inputs.composition.mounts,
            eboot_dirs: &inputs.composition.eboot_dirs,
            identity: &inputs.composition.identity,
            selection: selection.as_args(),
            strict_reserved: args.strict_reserved,
            checkpoint_override: args.checkpoint,
            budget_override: args.budget.map(Budget::new),
            prescan: args.prescan,
            guest_args: &args.guest_arg,
            check_anchor: !gate_args.no_anchor_check,
            // The set overwrites this with each child's own index.
            run_index: 0,
        },
        game::ThroughputPolicy {
            runs: gate_args.runs,
            strict: gate_args.strict_perf,
        },
        &*sink,
    ) {
        Ok(o) => o,
        Err(game::SpawnError::Command(error)) => {
            bar.abort();
            return Err(error);
        }
        Err(error) => {
            bar.abort();
            let e = separate_spawn_command_error(error)?;
            eprintln!("boot bench: {e}");
            let captured_stdout = e.captured_stdout();
            if !captured_stdout.is_empty() {
                eprintln!("stdout:\n{captured_stdout}");
            }
            let captured_stderr = e.captured_stderr();
            if !captured_stderr.is_empty() {
                eprintln!("stderr:\n{captured_stderr}");
            }
            return Ok(CommandExitCode::new(EXIT_SUBPROCESS_FAIL));
        }
    };
    bar.finish();
    match outcome.gate {
        game::BenchGate::Pass => {}
        game::BenchGate::DeterminismBreak => {
            // A single run can move two witnesses, so a set of N runs
            // can report more than N disagreements.
            eprintln!(
                "boot bench: {} disagreement(s) across the {} run(s) of the set:",
                outcome.determinism_failures.len(),
                outcome.runs.len(),
            );
            for failure in &outcome.determinism_failures {
                eprintln!("  {failure}");
            }
            eprintln!(
                "the runs took identical inputs, so a disagreement is a determinism \
                 defect. The report on stdout gives one of three things: the first step \
                 two traced re-runs diverge at, the commands that find it, or the reason \
                 the localization could not run. \
                 exiting with status {EXIT_DETERMINISM_BREAK}"
            );
            return Ok(CommandExitCode::new(EXIT_DETERMINISM_BREAK));
        }
        game::BenchGate::AnchorDrift => {
            let game::AnchorVerdict::Drift(failures) = &outcome.anchor else {
                return Err(CommandError::failed(
                    "boot bench: anchor-drift gate carried no drift verdict",
                ));
            };
            let cell = plan
                .cell
                .as_ref()
                .map_or_else(String::new, |c| format!(" {}", c.label()));
            eprintln!(
                "boot bench: {} disagreement(s) with the committed anchor for {} \
                 (content id {}{cell}):",
                failures.len(),
                inputs.title.name(),
                inputs.title.content_id,
            );
            for failure in failures {
                eprintln!("  {failure}");
            }
            eprintln!(
                "this run used the configuration the cell's anchor was recorded under, so \
                 the movement is a regression until it is attributed to a change. \
                 Once it is, re-bless with:\n  \
                 cargo run --release -p cellgov_cli -- dev record-anchors --title {}\n\
                 --no-anchor-check drops this gate for a measurement-only run.\n\
                 exiting with status {EXIT_ANCHOR_DRIFT}",
                inputs.title.name(),
            );
            return Ok(CommandExitCode::new(EXIT_ANCHOR_DRIFT));
        }
        game::BenchGate::SpreadExceeded => {
            let detail = match outcome.throughput {
                game::ThroughputVerdict::Inconclusive { spread_pct, .. } => format!(
                    "the {} runs spread {spread_pct:.2}%, above the \
                     {:.1}% ceiling",
                    outcome.runs.len(),
                    game::BENCH_SPREAD_CEILING_PCT,
                ),
                game::ThroughputVerdict::Unmeasurable => {
                    "a run reported a zero wall, so there is no spread to compare".to_string()
                }
                game::ThroughputVerdict::Measured { .. } => {
                    return Err(CommandError::failed(
                        "boot bench: spread-exceeded gate carried a measured verdict",
                    ))
                }
            };
            eprintln!(
                "boot bench: --strict-perf: no throughput verdict -- {detail}. \
                 Without --strict-perf this reports and exits 0: elapsed time on a host \
                 running anything else measures the host. \
                 exiting with status {EXIT_SPREAD_EXCEEDED}"
            );
            return Ok(CommandExitCode::new(EXIT_SPREAD_EXCEEDED));
        }
    }
    Ok(CommandExitCode::SUCCESS)
}
