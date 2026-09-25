//! `boot run`: the run itself and its exit classification.

use std::path::Path;

use cellgov_compare::BootOutcome;
use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressBar;
use cellgov_time::Budget;

use crate::game;
use crate::progress::RUN_TASK;
use cellgov_boot::compose::{ExecutionOverrides, ResolvedPlan};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::BootRunArgs;
use crate::cli::title::resolve_ps3_vfs_root;

use super::compose::firmware_module_dir;
use super::inputs::resolve_boot_inputs;

/// `boot run` terminated with a guest fault.
const EXIT_RUN_GAME_FAULT: i32 = exit_codes::command_specific(10);
/// `boot run` reached `--max-steps` without hitting the configured
/// checkpoint.
const EXIT_RUN_GAME_MAX_STEPS: i32 = exit_codes::command_specific(11);
/// `boot run` exhausted simulated time before reaching a terminal
/// state.
const EXIT_RUN_GAME_TIME_OVERFLOW: i32 = exit_codes::command_specific(12);
/// `boot run` completed but the loop logged an anomaly that violates
/// the determinism contract (lost syscall-wake responses).
const EXIT_RUN_GAME_CRITICAL_ANOMALY: i32 = exit_codes::command_specific(13);
/// `boot run` failed to save a requested `--save-observation` /
/// `--save-boot-summary` artifact; `ObservationSaveError` says which
/// failures leave a partial file. A `--save-state-trace` write failure
/// takes the shared failed status instead.
const EXIT_RUN_GAME_SAVE_ARTIFACT: i32 = exit_codes::command_specific(14);

pub(crate) fn run_game(
    args: &BootRunArgs,
    vfs_flag: Option<&Path>,
    render: RenderFlags,
) -> Result<CommandExitCode, CommandError> {
    let observation_regions = match args.observation_manifest.as_deref() {
        Some(path) => Some(
            cellgov_compare::checkpoint_manifest::load(Path::new(path))
                .map_err(|error| CommandError::failed(format!("--observation-manifest: {error}")))?
                .region_descriptors(),
        ),
        None => None,
    };
    let vfs_root = resolve_ps3_vfs_root(vfs_flag)?;
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        args.overrides.overrides(),
        &vfs_root,
        args.elf_path.as_deref(),
        "boot run",
    )?;
    let firmware_dir = firmware_module_dir(&inputs.composition)?;
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    // `boot run` drives `step_loop` at the title's checkpoint.
    let ends_at_cell_checkpoint = cellgov_boot::step_loop::step_loop_ends_at(
        inputs.title.checkpoint_trigger(),
        plan.checkpoint,
    );
    let finish_line = game::anchor_finish_line(
        &inputs.title.content_id,
        plan.cell.as_ref(),
        run_retargets_anchor(args, !ends_at_cell_checkpoint),
    );
    let bar = ProgressBar::start(render.caps(), &RUN_TASK, inputs.title.name());
    let sink = bar.sink();
    let result = game::run_game(
        game::RunExecution {
            title: cellgov_boot::prepare::TitleOptions {
                manifest: &inputs.title,
                elf_path: &inputs.elf_path,
                elf_data: inputs.elf_data,
                authority_id: inputs.authority_id,
                control_flags1: inputs.control_flags1,
                firmware_dir: firmware_dir.as_deref(),
                composed_mounts: &inputs.composition.mounts,
                eboot_dirs: &inputs.composition.eboot_dirs,
                identity: &inputs.composition.identity,
            },
            limits: cellgov_boot::prepare::ExecutionOptions {
                runtime_max_steps: args.max_steps,
                budget_override: args.budget.map(Budget::new),
                strict_reserved: args.strict_reserved,
                capture_state_trace: args.save_state_trace.is_some(),
                guest_args: &args.guest_arg,
                patch_bytes: args.patch_byte.as_deref().unwrap_or(&[]),
            },
        },
        game::RunArtifacts {
            observation: args.save_observation.as_deref(),
            observation_regions: observation_regions.as_deref(),
            boot_summary: args.save_boot_summary.as_deref(),
            state_trace: args.save_state_trace.as_deref(),
        },
        game::RunReporting {
            boot: cellgov_boot::prepare::DiagnosticOptions {
                print_banner: true,
                prescan: args.prescan,
                profile_pairs: args.profile_pairs,
                dump_at_pc: args.dump_at_pc,
                dump_skip: args.dump_skip,
                dump_mem_boot_addrs: args.dump_mem_boot.as_deref().unwrap_or(&[]),
                dump_mem_fault_ranges: args.dump_mem_fault.as_deref().unwrap_or(&[]),
            },
            trace: args.trace,
            profile: args.profile,
            state_hash_census: args.state_hash_census,
            progress: &*sink,
            finish_line,
        },
    );
    let summary = match result {
        Ok(s) => {
            bar.finish();
            s
        }
        Err(e) => {
            bar.abort();
            if e.is_report_artifact_failure() {
                return Err(CommandError::status(
                    EXIT_RUN_GAME_SAVE_ARTIFACT,
                    format!("boot run: {e}"),
                ));
            }
            return Err(boot_run_error(&e));
        }
    };
    let code = classify_run_game_exit(&summary);
    Ok(CommandExitCode::new(code))
}

pub(super) fn boot_run_error(error: &impl std::fmt::Display) -> CommandError {
    CommandError::failed(format!("boot run: {error}"))
}

/// Whether a `boot run` flag moves the run off the trajectory its
/// cell's anchor recorded, so the anchor's step count is not where
/// this run ends.
///
/// `checkpoint_elsewhere` says the cell's anchor recorded a checkpoint
/// this run does not stop at; see
/// [`cellgov_boot::step_loop::step_loop_ends_at`]. The
/// cap is a ceiling the run may stop under, and moves nothing. The
/// diagnostic flags change only what the run prints. `--dump-at-pc`
/// ends the run at its break.
///
/// The inputs `boot bench` also takes are judged by
/// [`ExecutionOverrides::trajectory_overrides`], the rule both commands
/// share.
pub(super) fn run_retargets_anchor(args: &BootRunArgs, checkpoint_elsewhere: bool) -> bool {
    let overrides = args.overrides.overrides();
    let shared = ExecutionOverrides {
        budget: args.budget.map(Budget::new),
        strict_reserved: args.strict_reserved,
        guest_args: &args.guest_arg,
        boot: &overrides,
    };
    args.elf_path.is_some()
        || !shared.trajectory_overrides().is_empty()
        || args.patch_byte.as_ref().is_some_and(|p| !p.is_empty())
        || args.dump_at_pc.is_some()
        || checkpoint_elsewhere
}

/// Map a [`game::RunSummary`] to a process exit code. A critical
/// anomaly (lost syscall-wake response) overrides a clean outcome.
fn classify_run_game_exit(summary: &game::RunSummary) -> i32 {
    let outcome_code = match summary.outcome {
        BootOutcome::ProcessExit | BootOutcome::RsxWriteCheckpoint | BootOutcome::PcReached(_) => 0,
        BootOutcome::Fault => EXIT_RUN_GAME_FAULT,
        BootOutcome::MaxSteps => EXIT_RUN_GAME_MAX_STEPS,
        BootOutcome::TimeOverflow => EXIT_RUN_GAME_TIME_OVERFLOW,
    };
    if outcome_code == 0 && summary.had_critical_anomaly {
        return EXIT_RUN_GAME_CRITICAL_ANOMALY;
    }
    outcome_code
}

#[cfg(test)]
#[path = "tests/boot_run_finish_line_tests.rs"]
mod boot_run_finish_line_tests;

#[cfg(test)]
#[path = "tests/boot_run_override_finish_line_tests.rs"]
mod boot_run_override_finish_line_tests;
