//! Schedule exploration over a window of a composed title boot.
//!
//! This command composes the cell the way the boot family composes it.
//! It then drives the prepared runtime to the window's start and hands
//! what remains to [`cellgov_explore::explore_window`]. The explorer
//! builds one runtime and reaches each alternate through a snapshot, so
//! a title-scale exploration costs one composition rather than one per
//! schedule.

use std::path::Path;
use std::rc::Rc;

use cellgov_boot::manifest::CheckpointTrigger;
use cellgov_boot::prepare::{
    prepare, BootServices, DiagnosticOptions, ExecutionOptions, PrepareOptions, PreparedBoot,
    TitleOptions,
};
use cellgov_boot::step_loop::rsx_checkpoint_addr;
use cellgov_boot::BootSink;
use cellgov_compare::BootOverrides;
use cellgov_core::Runtime;
use cellgov_explore::{
    open_window, ExplorationConfig, ExplorationResult, OutcomeClass, StopClass, StopReason,
    WindowStart,
};

use super::window::{never_opened, start_past_cap};
use crate::cli::boot_cmd::{anchor_plan, firmware_module_dir, plan_max_steps, resolve_boot_inputs};
use crate::cli::compare::report_first_invariant_break;
use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::parse::{ExploreTitleArgs, OutputFormat};
use crate::cli::title::resolve_ps3_vfs_root;
use cellgov_boot::compose::ResolvedPlan;

/// The subcommand name every refusal below carries.
const SUBCMD: &str = "explore title";

/// Exit code: the model refused a schedule the exploration asked for.
const EXIT_MODEL_REFUSAL: i32 = exit_codes::command_specific(20);

/// Exit code: the boot ended before the window's start condition held.
const EXIT_WINDOW_NEVER_OPENED: i32 = exit_codes::command_specific(21);

/// Exit code: a schedule the exploration ran ended in a guest fault.
const EXIT_GUEST_FAULT: i32 = exit_codes::command_specific(22);

/// Exit code: the window holds something the search cannot reason
/// about, so it answers for none of it.
const EXIT_WINDOW_UNSERVED: i32 = exit_codes::command_specific(23);

/// Exit code: the window is schedule-sensitive, the same verdict the
/// scenario and microtest entry points give.
const EXIT_SCHEDULE_SENSITIVE: i32 = exit_codes::FAILED;

/// A boot's narration, on stderr so `--format json` leaves one document
/// on stdout.
struct StderrSink;

impl BootSink for StderrSink {
    fn note(&self, line: &str) {
        eprintln!("{line}");
    }

    fn warn(&self, line: &str) {
        eprintln!("{line}");
    }

    fn guest_text(&self, text: &str) {
        eprint!("{text}");
    }
}

pub(super) fn run(
    args: &ExploreTitleArgs,
    format: OutputFormat,
    vfs_flag: Option<&Path>,
) -> Result<CommandExitCode, CommandError> {
    let start = window_start(args);
    let vfs_root = resolve_ps3_vfs_root(vfs_flag)?;
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        // The command composes the cell the anchor names and changes
        // nothing about the boot, so it declares no override.
        BootOverrides::default(),
        &vfs_root,
        None,
        SUBCMD,
    )?;
    let firmware_dir = firmware_module_dir(&inputs.composition)?;
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    // Without a cap of its own the exploration takes the cell's, so a
    // window opens over the same prefix the anchor covers.
    let max_steps = match args.max_steps {
        Some(max_steps) => max_steps,
        None => plan_max_steps(&plan, &inputs.title)?,
    };
    let checkpoint = anchor_plan(&plan).checkpoint;
    let mut rt = prepared_runtime(&inputs, firmware_dir.as_deref(), max_steps)?;
    // Both counts are runtime steps; see the doc on `WindowStart::Step`.
    if let Some(refusal) = start_past_cap(start, rt.max_steps()) {
        // A flag value this boot cannot satisfy is a usage error.
        return Err(CommandError::status(exit_codes::USAGE, refusal));
    }

    let opened_at = match open_window(&mut rt, start) {
        Ok(step) => step,
        Err(error) => {
            return Err(CommandError::status(
                EXIT_WINDOW_NEVER_OPENED,
                never_opened(&error, checkpoint),
            ))
        }
    };

    let config = ExplorationConfig {
        max_schedules: args.max_schedules,
        max_steps_per_run: args.max_steps_per_run,
    };
    let result = cellgov_explore::explore_window(move || rt, &config);

    report_first_invariant_break(result.first_invariant_break.as_deref());
    let window = Window {
        title: inputs.title.name().to_string(),
        start,
        opened_at,
        checkpoint,
    };
    match format {
        OutputFormat::Human => print!("{}", human(&window, &result)),
        OutputFormat::Json => println!("{}", json(&window, &result)?),
    }
    Ok(CommandExitCode::new(exit_code(&window, &result)))
}

/// The start condition the flags name.
fn window_start(args: &ExploreTitleArgs) -> WindowStart {
    // Clap's group makes the two flags exclusive, so the first arm
    // never hides a `--start-pc`.
    match (args.start_step, args.start_pc) {
        (Some(n), _) => WindowStart::Step(n),
        (_, Some(pc)) => WindowStart::Pc(pc),
        (None, None) => WindowStart::FirstBranchingPoint,
    }
}

/// Bring the title to a runtime one `step()` from its first instruction.
///
/// No staged child-init pass runs here or in the explorer's replays;
/// [`open_window`] refuses the first one it meets.
fn prepared_runtime(
    inputs: &crate::cli::boot_cmd::BootInputs,
    firmware_dir: Option<&str>,
    max_steps: usize,
) -> Result<Runtime, CommandError> {
    let prepared: PreparedBoot = prepare(PrepareOptions {
        title: TitleOptions {
            manifest: &inputs.title,
            elf_path: &inputs.elf_path,
            elf_data: inputs.elf_data.clone(),
            authority_id: inputs.authority_id,
            control_flags1: inputs.control_flags1,
            firmware_dir,
            composed_mounts: &inputs.composition.mounts,
            eboot_dirs: &inputs.composition.eboot_dirs,
            identity: &inputs.composition.identity,
        },
        execution: ExecutionOptions {
            runtime_max_steps: max_steps,
            budget_override: None,
            strict_reserved: false,
            capture_state_trace: false,
            guest_args: &[],
            patch_bytes: &[],
        },
        diagnostics: DiagnosticOptions {
            print_banner: true,
            prescan: false,
            profile_pairs: false,
            dump_at_pc: None,
            dump_skip: 0,
            dump_mem_boot_addrs: &[],
            dump_mem_fault_ranges: &[],
        },
        services: BootServices {
            sink: Rc::new(StderrSink),
            keys: Rc::new(crate::cli::keys::ProcessKeyVault),
            taps: Rc::new(cellgov_boot::NoTaps),
        },
    })
    .map_err(|error| CommandError::failed(format!("{SUBCMD}: {error}")))?;
    Ok(prepared.rt)
}

/// Which part of the boot a verdict covers.
struct Window {
    title: String,
    start: WindowStart,
    /// Runtime-step count the window opened at.
    opened_at: usize,
    /// Where the cell's anchor stops the boot.
    checkpoint: CheckpointTrigger,
}

impl Window {
    /// The step the window ends at, given what the baseline did inside
    /// it.
    ///
    /// `baseline_steps` counts committed steps. A refused commit is one
    /// more step the runtime took, so the window reaches one past the
    /// hash.
    fn closed_at(&self, result: &ExplorationResult) -> usize {
        let uncommitted = usize::from(matches!(result.baseline_stop, StopReason::CommitError(_)));
        self.opened_at
            .saturating_add(result.baseline_steps)
            .saturating_add(uncommitted)
    }

    /// The guest address `stop` is the cell's checkpoint at, or `None`
    /// when it is some other stop.
    ///
    /// A `first-rsx-write` checkpoint reaches this driver as a refused
    /// commit, so only the cell's trigger separates it from a refusal.
    fn checkpoint_addr(&self, stop: StopReason) -> Option<u64> {
        match stop {
            StopReason::CommitError(e) => rsx_checkpoint_addr(self.checkpoint, e),
            _ => None,
        }
    }

    /// Whether `stop` is a refusal the cell's checkpoint does not
    /// explain.
    fn is_model_refusal(&self, stop: StopReason) -> bool {
        stop.class() == StopClass::Refusal && self.checkpoint_addr(stop).is_none()
    }

    /// How many refusals in `result` the cell's checkpoint does not
    /// explain, the baseline included.
    fn model_refusals(&self, result: &ExplorationResult) -> usize {
        self.count_stops(result, |stop| self.is_model_refusal(stop))
    }

    /// How many schedules in `result` the search would not answer for,
    /// the baseline included.
    fn unserved(&self, result: &ExplorationResult) -> usize {
        self.count_stops(result, |stop| stop.class() == StopClass::Unserved)
    }

    /// How many schedules in `result` ended in a guest fault, the
    /// baseline included.
    ///
    /// No checkpoint trigger ends a boot in a fault, so unlike
    /// [`Self::model_refusals`] this counts every one.
    fn guest_faults(&self, result: &ExplorationResult) -> usize {
        self.count_stops(result, |stop| stop.class() == StopClass::Fault)
    }

    fn count_stops(
        &self,
        result: &ExplorationResult,
        mut names: impl FnMut(StopReason) -> bool,
    ) -> usize {
        result.stops().filter(|stop| names(*stop)).count()
    }
}

/// The status the run exits with.
///
/// A refusal, a fault or an unserved schedule outranks the verdict, in
/// that order: a schedule that stopped short leaves the classification
/// incomplete. A bound and the cell's checkpoint both exit clean.
fn exit_code(window: &Window, result: &ExplorationResult) -> i32 {
    if window.model_refusals(result) > 0 {
        return EXIT_MODEL_REFUSAL;
    }
    if window.guest_faults(result) > 0 {
        return EXIT_GUEST_FAULT;
    }
    if window.unserved(result) > 0 {
        return EXIT_WINDOW_UNSERVED;
    }
    match result.outcome {
        OutcomeClass::ScheduleSensitive => EXIT_SCHEDULE_SENSITIVE,
        OutcomeClass::ScheduleStable | OutcomeClass::Inconclusive => 0,
    }
}

/// The lines that name the window, ahead of the exploration's own
/// report.
fn window_lines(window: &Window, result: &ExplorationResult) -> String {
    // A step-count start already names the step it opened at.
    let at = match window.start {
        WindowStart::Step(_) => String::new(),
        _ => format!(" (step {})", window.opened_at),
    };
    let mut out = format!(
        "title: {}\nwindow_start: {}{at}\nwindow: steps {}..{}\n",
        window.title,
        window.start,
        window.opened_at,
        window.closed_at(result),
    );
    if let Some(addr) = window.checkpoint_addr(result.baseline_stop) {
        out.push_str(&format!(
            "window_end: the cell's {} checkpoint, at 0x{addr:08x}; the commit refusal \
             below is how the boot reaches it\n",
            window.checkpoint.as_cli_str(),
        ));
    }
    // An alternate stops at `--max-steps-per-run`, which can fall short
    // of the window. A truncated baseline already reports that through
    // its own `baseline_stop` line.
    if !result.baseline_stop.is_truncated() && result.schedules_truncated > 0 {
        out.push_str(&format!(
            "window_covered: {} of {} alternate(s) stopped before the window closed; each \
             stop is named below\n",
            result.schedules_truncated,
            result.schedules.len(),
        ));
    }
    out.push_str(&format!(
        "model_refusals: {}\n",
        window.model_refusals(result)
    ));
    // The number the guest-fault status comes from: the exploration's
    // own tallies below count refusals and truncations, not faults.
    out.push_str(&format!("guest_faults: {}\n", window.guest_faults(result)));
    let unserved = window.unserved(result);
    out.push_str(&format!("unserved: {unserved}\n"));
    if unserved > 0 {
        out.push_str(
            "the window holds a spawn whose staged init pass no exploration runs, \
             so the search answers for none of it; start the window after the spawn \
             or end it before\n",
        );
    }
    out
}

fn human(window: &Window, result: &ExplorationResult) -> String {
    let mut out = window_lines(window, result);
    out.push_str(&cellgov_explore::report::format_human(result));
    if result.total_branching_points == 0 {
        out.push_str(
            "note: no step in the window had a second runnable unit, so the window has \
             one schedule\n",
        );
    }
    out
}

fn json(window: &Window, result: &ExplorationResult) -> Result<String, CommandError> {
    let exploration: serde_json::Value =
        serde_json::from_str(&cellgov_explore::report::format_json(result)).map_err(|error| {
            CommandError::failed(format!("explore title: decode JSON: {error}"))
        })?;
    let doc = serde_json::json!({
        "title": window.title,
        "window": {
            "start": window.start.to_string(),
            "opened_at": window.opened_at,
            "closed_at": window.closed_at(result),
            "ended_at_checkpoint": window
                .checkpoint_addr(result.baseline_stop)
                .map(|addr| format!("0x{addr:08x}")),
        },
        "model_refusals": window.model_refusals(result),
        "guest_faults": window.guest_faults(result),
        "unserved": window.unserved(result),
        "exploration": exploration,
    });
    serde_json::to_string_pretty(&doc)
        .map_err(|error| CommandError::failed(format!("explore title: encode JSON: {error}")))
}

#[cfg(test)]
#[path = "tests/title_tests.rs"]
mod tests;
