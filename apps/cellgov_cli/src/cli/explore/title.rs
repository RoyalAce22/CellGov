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
use cellgov_explore::{ExplorationConfig, ExplorationResult, OutcomeClass, StopClass, StopReason};

use super::window::{open_window, start_past_cap, WindowStart};
use crate::cli::boot_cmd::{firmware_module_dir, resolve_boot_inputs, ResolvedPlan};
use crate::cli::compare::report_first_invariant_break;
use crate::cli::exit::die;
use crate::cli::exit_codes;
use crate::cli::parse::{die_usage, ExploreTitleArgs, OutputFormat};
use crate::cli::title::resolve_ps3_vfs_root;

/// The subcommand name every refusal below carries.
const SUBCMD: &str = "explore title";

/// Exit code: the model refused a schedule the exploration asked for.
const EXIT_MODEL_REFUSAL: i32 = exit_codes::command_specific(20);

/// Exit code: the boot ended before the window's start condition held.
const EXIT_WINDOW_NEVER_OPENED: i32 = exit_codes::command_specific(21);

/// Exit code: a schedule the exploration ran ended in a guest fault.
///
/// Separate from [`EXIT_MODEL_REFUSAL`] because the two name different
/// findings: a refusal is the model declining a step, and a fault is
/// the guest's own step failing. `explore window` calls the same stop a
/// fault, and this is how `explore title` says it.
const EXIT_GUEST_FAULT: i32 = exit_codes::command_specific(22);

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

pub(super) fn run(args: &ExploreTitleArgs, format: OutputFormat, vfs_flag: Option<&Path>) {
    let start = window_start(args);
    let vfs_root = resolve_ps3_vfs_root(vfs_flag);
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        // The command composes the cell the anchor names and changes
        // nothing about the boot, so it declares no override.
        BootOverrides::default(),
        &vfs_root,
        None,
        SUBCMD,
    );
    let firmware_dir = firmware_module_dir(&inputs.composition);
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    // Without a cap of its own the exploration takes the cell's, so a
    // window opens over the same prefix the anchor covers.
    let max_steps = args
        .max_steps
        .unwrap_or_else(|| plan.max_steps_usize(&inputs.title));
    let checkpoint = plan.as_plan().checkpoint;
    let mut rt = prepared_runtime(&inputs, firmware_dir.as_deref(), max_steps);
    crate::game::configure_rsx_from_manifest(&mut rt, &inputs.title);
    // `--start-step` counts runtime steps, which is what the runtime's
    // own cap bounds; see `WindowStart::Step`.
    if let Some(refusal) = start_past_cap(start, rt.max_steps()) {
        // A flag value this boot cannot satisfy is a usage error, so it
        // takes the shared usage status rather than the one a
        // schedule-sensitive window exits with.
        die_usage(&refusal);
    }

    let opened_at = match open_window(&mut rt, start, checkpoint) {
        Ok(step) => step,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(EXIT_WINDOW_NEVER_OPENED);
        }
    };

    let config = ExplorationConfig {
        max_schedules: args.max_schedules,
        max_steps_per_run: args.max_steps_per_run,
    };
    // `explore_window` takes the runtime the boot already built. The
    // factory hands over that one, and the explorer calls it once.
    let mut once = Some(rt);
    let result = cellgov_explore::explore_window(
        || {
            once.take()
                .expect("invariant: cellgov_explore::explore_window builds one runtime")
        },
        &config,
    );

    report_first_invariant_break(result.first_invariant_break.as_deref());
    let window = Window {
        title: inputs.title.name().to_string(),
        start,
        opened_at,
        checkpoint,
    };
    match format {
        OutputFormat::Human => print!("{}", human(&window, &result)),
        OutputFormat::Json => println!("{}", json(&window, &result)),
    }
    std::process::exit(exit_code(&window, &result));
}

/// The start condition the flags name.
fn window_start(args: &ExploreTitleArgs) -> WindowStart {
    match (args.start_step, args.start_pc) {
        (Some(n), _) => WindowStart::Step(n),
        (_, Some(pc)) => WindowStart::Pc(pc),
        // Clap's group makes the two flags exclusive, so this arm means
        // the caller gave neither.
        (None, None) => WindowStart::FirstBranchingPoint,
    }
}

/// Bring the title to a runtime one `step()` from its first
/// instruction. A refusal ends the process.
///
/// The staged child-init plans go unrun: this command's drive does not
/// run them, and neither do the explorer's replays. [`open_window`]
/// refuses by name the moment a child parks behind one, so it catches a
/// spawn before the window opens. A spawn inside the window escapes
/// that check.
fn prepared_runtime(
    inputs: &crate::cli::boot_cmd::BootInputs,
    firmware_dir: Option<&str>,
    max_steps: usize,
) -> Runtime {
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
    .unwrap_or_else(|e| die(&format!("{SUBCMD}: {e}")));
    prepared.rt
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
    /// `baseline_steps` counts committed steps. A refused commit is a
    /// step the runtime took and did not commit. The window then
    /// reaches one step further than the hash covers. A cell whose
    /// checkpoint is a write into the reserved RSX region takes that
    /// path.
    fn closed_at(&self, result: &ExplorationResult) -> usize {
        let uncommitted = usize::from(matches!(result.baseline_stop, StopReason::CommitError(_)));
        self.opened_at
            .saturating_add(result.baseline_steps)
            .saturating_add(uncommitted)
    }

    /// The guest address `stop` is the cell's checkpoint at, or `None`
    /// when it is some other stop.
    ///
    /// A `first-rsx-write` checkpoint reaches a driver as a refused
    /// commit, the same shape a defect in the commit pipeline takes.
    /// Only the cell's declared trigger separates the two.
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

    /// How many schedules in `result` ended in a guest fault, the
    /// baseline included.
    ///
    /// No checkpoint explains a fault: a `first-rsx-write` cell reaches
    /// its checkpoint as a refused commit, and every other trigger as a
    /// stop no unit faulted on.
    fn guest_faults(&self, result: &ExplorationResult) -> usize {
        self.count_stops(result, |stop| stop.class() == StopClass::Fault)
    }

    fn count_stops(
        &self,
        result: &ExplorationResult,
        mut names: impl FnMut(StopReason) -> bool,
    ) -> usize {
        let baseline = usize::from(names(result.baseline_stop));
        let alternates = result.schedules.iter().filter(|s| names(s.stop)).count();
        baseline.saturating_add(alternates)
    }
}

/// The status the run exits with.
///
/// A refusal outranks the verdict: a schedule the model would not run
/// leaves the classification to the schedules that ran, so the refusal
/// is the finding. A guest fault outranks it for the same reason and
/// exits under its own name, because it says something about the guest
/// rather than about the model. A bound is the caller's own cap, and
/// the cell's checkpoint is where the boot stops; both exit clean.
fn exit_code(window: &Window, result: &ExplorationResult) -> i32 {
    if window.model_refusals(result) > 0 {
        return EXIT_MODEL_REFUSAL;
    }
    if window.guest_faults(result) > 0 {
        return EXIT_GUEST_FAULT;
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
    // The explorer replays each alternate under `--max-steps-per-run`,
    // which is its own bound and shorter than the window whenever the
    // baseline ran past it. This line keeps the range above from reading
    // as the range every schedule covered. A baseline that stopped short
    // marks every record truncated on its own account, and the
    // exploration's `baseline_stop` line already carries that.
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

fn json(window: &Window, result: &ExplorationResult) -> String {
    let exploration: serde_json::Value =
        serde_json::from_str(&cellgov_explore::report::format_json(result))
            .expect("invariant: the exploration report is float-free by construction");
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
        "exploration": exploration,
    });
    serde_json::to_string_pretty(&doc)
        .expect("invariant: a primitive-only serde_json::Value tree always serializes")
}

#[cfg(test)]
#[path = "tests/title_tests.rs"]
mod tests;
