//! Dispatch for the boot-family subcommands: `boot run`,
//! `boot bench-once`, and `boot bench`.

use std::path::Path;

use cellgov_compare::BootOutcome;
use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressBar;
use cellgov_time::Budget;

use crate::composition::{
    banner, compose_boot, BootComposition, ComposeError, ComposeInputs, FirmwareChoice, GameChoice,
    GameVersion,
};
use crate::game;
use crate::game::manifest::{CellKey, BASE_GAME_VER};
use crate::progress::{BENCH_PAIR_TASK, BENCH_TASK, RUN_TASK};

use super::env::parse_env_bool;
use super::exit::{die, LoadedPpuImage, TitleNotInstalled};
use super::exit_codes;
use super::parse::{
    die_usage, BenchArgs, BenchGateArgs, BootRunArgs, BootSelection, TitleSelector,
};
use super::title::{resolve_ps3_vfs_root, resolve_title_manifest};
use crate::paths::{cell_checkpoint, cell_max_steps};

/// The `sys/external` modules inside a firmware entry, relative to the
/// entry's `dev_flash` mount.
const FIRMWARE_EXTERNAL: [&str; 2] = ["sys", "external"];

/// Set to `1` by synthetic harnesses (e.g. ps3autotests) to suppress
/// the auto-default.
const DISABLE_DEFAULT_ENV: &str = "CELLGOV_NO_FIRMWARE_DIR";

/// Exit code: the runs of a set disagreed on step count, outcome or a
/// witness.
const EXIT_DETERMINISM_BREAK: i32 = exit_codes::DISAGREED;

/// Exit code: `--strict-perf` is set and the run set reaches no
/// throughput verdict.
pub(super) const EXIT_SPREAD_EXCEEDED: i32 = exit_codes::command_specific(15);

/// Exit code: a bench subprocess failed or its `BENCH_RESULT` line was
/// unparseable.
const EXIT_SUBPROCESS_FAIL: i32 = exit_codes::DIVERGED;

/// Exit code: the run disagreed with the title's committed anchor.
const EXIT_ANCHOR_DRIFT: i32 = exit_codes::ANCHOR_MOVED;

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

/// Resolve `--fw`, `--game-ver` and `--firmware-dir` against the
/// store, then print the selection banner before any other output.
///
/// An unresolved selection is fatal here. A title that boots without
/// firmware binds no import and dies dozens of steps later naming a
/// NID, which says nothing about the firmware.
pub(super) fn resolve_composition(
    selection: &BootSelection,
    vfs_root: &Path,
    title: &game::manifest::TitleManifest,
) -> BootComposition {
    try_resolve_composition(selection, vfs_root, title)
        .unwrap_or_else(|e| die(&format!("boot: {e}")))
}

/// [`resolve_composition`] with the refusal returned, so a sweep can
/// name the cell it stops and continue to the next.
///
/// # Errors
///
/// Every [`ComposeError`]; the banner prints only for a composition
/// that resolved.
pub(super) fn try_resolve_composition(
    selection: &BootSelection,
    vfs_root: &Path,
    title: &game::manifest::TitleManifest,
) -> Result<BootComposition, ComposeError> {
    if let Some(explicit) = &selection.firmware_dir {
        if !explicit.is_dir() {
            die(&format!(
                "--firmware-dir: {} is not an existing directory",
                explicit.display()
            ));
        }
    }
    let install_root = super::keys::install_root_of(vfs_root);
    let composition = compose_boot(&ComposeInputs {
        title,
        vfs_root,
        install_root: &install_root,
        fw: selection.fw.as_deref(),
        game_ver: selection.game_ver.as_deref(),
        firmware_dir: selection.firmware_dir.as_deref(),
        // The value decides: `CELLGOV_NO_FIRMWARE_DIR=0` leaves the
        // default in place.
        no_firmware: parse_env_bool(DISABLE_DEFAULT_ENV),
        disable_env: DISABLE_DEFAULT_ENV,
    })?;
    for line in banner::render(title, &composition) {
        eprintln!("{line}");
    }
    for line in banner::render_firmware_notes(&composition.understated_firmware) {
        eprintln!("{line}");
    }
    // The machine form of the banner above, so a parent process that
    // spawned this boot can record what the run was measured against.
    match composition.identity.render_sentinel_line() {
        Ok(line) => eprintln!("{line}"),
        Err(e) => die(&format!("boot: serializing the run identity: {e}")),
    }
    Ok(composition)
}

/// The `sys/external` directory the firmware loader reads its modules
/// from, or `None` for a boot with no firmware.
pub(super) fn firmware_module_dir(composition: &BootComposition) -> Option<String> {
    let dir = match &composition.firmware {
        FirmwareChoice::Managed(entry) => FIRMWARE_EXTERNAL
            .iter()
            .fold(entry.dev_flash_dir(), |d, part| d.join(part)),
        // `--firmware-dir` names a `sys/external` tree directly, so
        // this arm joins nothing onto it.
        FirmwareChoice::Unmanaged { dir } => dir.clone(),
        FirmwareChoice::None => return None,
    };
    Some(
        dir.to_str()
            .unwrap_or_else(|| {
                die(&format!(
                    "boot: firmware module directory {} is not valid UTF-8",
                    dir.display()
                ))
            })
            .to_string(),
    )
}

/// The selection flags this process received, owned so a
/// [`game::SelectionArgs`] can borrow them across the call that
/// encodes a child invocation.
pub(super) struct OwnedSelection {
    fw: Option<String>,
    game_ver: Option<String>,
    firmware_dir: Option<String>,
    vfs_root: Option<String>,
}

impl OwnedSelection {
    pub(super) fn as_args(&self) -> game::SelectionArgs<'_> {
        game::SelectionArgs {
            fw: self.fw.as_deref(),
            game_ver: self.game_ver.as_deref(),
            firmware_dir: self.firmware_dir.as_deref(),
            vfs_root: self.vfs_root.as_deref(),
        }
    }
}

/// A path a child invocation must be able to spell back on its own
/// command line.
fn forwardable(path: Option<&Path>, flag: &str) -> Option<String> {
    path.map(|p| {
        p.to_str()
            .unwrap_or_else(|| {
                die(&format!(
                    "{flag} {} is not valid UTF-8, so a child run cannot be given it",
                    p.display()
                ))
            })
            .to_string()
    })
}

pub(super) fn selection_args(selection: &BootSelection, vfs_flag: Option<&Path>) -> OwnedSelection {
    OwnedSelection {
        fw: selection.fw.clone(),
        game_ver: selection.game_ver.clone(),
        firmware_dir: forwardable(selection.firmware_dir.as_deref(), "--firmware-dir"),
        vfs_root: forwardable(vfs_flag, "--vfs-root"),
    }
}

pub(super) struct BootInputs {
    pub(super) title: game::manifest::TitleManifest,
    /// What the store composed for this run: the firmware, the game
    /// version, and the guest tree the two produce.
    pub(super) composition: BootComposition,
    pub(super) elf_path: String,
    /// Pre-loaded plaintext ELF bytes from the loader (explicit
    /// path or candidate walk). Passed to `prepare()` so the
    /// decrypt happens exactly once.
    pub(super) elf_data: Vec<u8>,
    /// Program authority id from the SELF identification header;
    /// `None` for raw-ELF inputs (boot serves the retail fallback).
    pub(super) authority_id: Option<u64>,
    pub(super) control_flags1: Option<u32>,
}

fn resolve_boot_inputs(
    selector: &TitleSelector,
    selection: &BootSelection,
    vfs_root: &Path,
    explicit_elf: Option<&str>,
    subcmd: &str,
) -> BootInputs {
    let title = resolve_title_manifest(selector, subcmd);
    let composition = resolve_composition(selection, vfs_root, &title);
    let (elf_path, image) = match explicit_elf {
        Some(p) => {
            let image = crate::cli::exit::load_ppu_image_with_title_or_die(p, &title, vfs_root);
            (p.to_string(), image)
        }
        None => {
            let (image, path) = crate::cli::exit::load_ppu_image_walk_candidates_or_die(
                &title,
                vfs_root,
                &composition.eboot_dirs,
            );
            (forwardable_eboot_path(&path, subcmd), image)
        }
    };
    boot_inputs(title, composition, elf_path, image)
}

/// The inputs a sweep resolves for one declared cell, or the reason
/// the title's dump is not on this machine.
///
/// The composition is the caller's: the sweep composes each cell by
/// name and classifies a composition refusal itself, so this covers
/// the image walk alone.
///
/// # Errors
///
/// The dump is not on this machine. A candidate that exists and fails
/// to load dies in the walk.
pub(super) fn try_resolve_cell_inputs(
    title: game::manifest::TitleManifest,
    composition: BootComposition,
    vfs_root: &Path,
    subcmd: &str,
) -> Result<BootInputs, TitleNotInstalled> {
    let (image, path) = crate::cli::exit::load_ppu_image_walk_candidates(
        &title,
        vfs_root,
        &composition.eboot_dirs,
    )?;
    let elf_path = forwardable_eboot_path(&path, subcmd);
    Ok(boot_inputs(title, composition, elf_path, image))
}

/// A resolved EBOOT path as a child invocation spells it.
fn forwardable_eboot_path(path: &Path, subcmd: &str) -> String {
    path.to_str()
        .map(|s| s.replace('\\', "/"))
        .unwrap_or_else(|| {
            die(&format!(
                "{subcmd}: resolved EBOOT path is not valid UTF-8: {}",
                path.display()
            ))
        })
}

/// Announce the loaded image and assemble the inputs.
///
/// Past the line this prints, a run that fails is a boot failure and
/// never a missing dump; the suites key their skip/fail split on it.
fn boot_inputs(
    title: game::manifest::TitleManifest,
    composition: BootComposition,
    elf_path: String,
    image: LoadedPpuImage,
) -> BootInputs {
    // `eboot` and `elf_bytes` name the image that was actually loaded,
    // so a stale build cannot pass for a fresh one. `elf_bytes` counts
    // the plaintext ELF, which for a SELF input differs from the file
    // on disk; both are deterministic, unlike an mtime.
    eprintln!(
        "{} title={} eboot={} elf_bytes={}",
        cellgov_compare::witnesses::BOOT_STARTED_SENTINEL,
        title.name(),
        elf_path,
        image.elf_data.len(),
    );
    BootInputs {
        title,
        composition,
        elf_path,
        elf_data: image.elf_data,
        authority_id: image.authority_id,
        control_flags1: image.control_flags1,
    }
}

pub(crate) fn run_game(args: &BootRunArgs, vfs_flag: Option<&Path>, render: RenderFlags) {
    let observation_regions: Option<Vec<cellgov_compare::RegionDescriptor>> =
        args.observation_manifest.as_deref().map(|path| {
            cellgov_compare::checkpoint_manifest::load(Path::new(path))
                .unwrap_or_else(|e| die(&format!("--observation-manifest: {e}")))
                .region_descriptors()
        });
    let vfs_root = resolve_ps3_vfs_root(vfs_flag);
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        &vfs_root,
        args.elf_path.as_deref(),
        "boot run",
    );
    let firmware_dir = firmware_module_dir(&inputs.composition);
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    let ends_at_cell_checkpoint =
        run_ends_at_cell_checkpoint(plan.checkpoint, inputs.title.checkpoint_trigger());
    let finish_line = game::anchor_finish_line(
        &inputs.title.content_id,
        plan.cell.as_ref(),
        run_retargets_anchor(args, !ends_at_cell_checkpoint),
    );
    let bar = ProgressBar::start(render.caps(), &RUN_TASK, inputs.title.name());
    let sink = bar.sink();
    let result = game::run_game(game::RunGameOptions {
        title: &inputs.title,
        elf_path: &inputs.elf_path,
        elf_data: inputs.elf_data,
        authority_id: inputs.authority_id,
        control_flags1: inputs.control_flags1,
        max_steps: args.max_steps,
        trace: args.trace,
        profile: args.profile,
        firmware_dir: firmware_dir.as_deref(),
        composed_mounts: &inputs.composition.mounts,
        identity: &inputs.composition.identity,
        dump_at_pc: args.dump_at_pc,
        dump_skip: args.dump_skip,
        patch_bytes: args.patch_byte.as_deref().unwrap_or(&[]),
        dump_mem_boot_addrs: args.dump_mem_boot.as_deref().unwrap_or(&[]),
        dump_mem_fault_ranges: args.dump_mem_fault.as_deref().unwrap_or(&[]),
        save_observation: args.save_observation.as_deref(),
        observation_regions: observation_regions.as_deref(),
        save_boot_summary: args.save_boot_summary.as_deref(),
        save_state_trace: args.save_state_trace.as_deref(),
        strict_reserved: args.strict_reserved,
        profile_pairs: args.profile_pairs,
        budget_override: args.budget.map(Budget::new),
        prescan: args.prescan,
        guest_args: &args.guest_arg,
        progress: &*sink,
        finish_line,
    });
    // Down before any exit: `process::exit` runs no destructor, so a
    // bar left standing keeps its render thread and a hidden cursor.
    // The failure arm takes `abort`, which flags the terminal-native
    // progress state as an error instead of clearing it as a run that
    // completed.
    let summary = match result {
        Ok(s) => {
            bar.finish();
            s
        }
        Err(e) => {
            bar.abort();
            eprintln!("boot run: {e}");
            std::process::exit(EXIT_RUN_GAME_SAVE_ARTIFACT);
        }
    };
    let code = classify_run_game_exit(&summary);
    if code != 0 {
        std::process::exit(code);
    }
}

/// Whether `boot run` ends at the checkpoint the cell's anchor recorded.
///
/// `boot run` stops at the title's checkpoint. Its driver sets no
/// target PC (`step_loop::driver` classifies every step with none), so
/// the run passes a `pc=` checkpoint and does not stop there. An anchor
/// recorded at one is therefore not where this run ends, even when the
/// title declares the same checkpoint.
fn run_ends_at_cell_checkpoint(
    cell_checkpoint: game::manifest::CheckpointTrigger,
    title_checkpoint: game::manifest::CheckpointTrigger,
) -> bool {
    cell_checkpoint == title_checkpoint
        && !matches!(cell_checkpoint, game::manifest::CheckpointTrigger::Pc(_))
}

/// Whether a `boot run` flag moves the run off the trajectory its
/// cell's anchor recorded, so the anchor's step count is not where
/// this run ends.
///
/// `checkpoint_elsewhere` says the cell's anchor recorded a checkpoint
/// this run does not stop at; see [`run_ends_at_cell_checkpoint`]. The
/// cap is a ceiling the run may stop under, and moves nothing. The
/// diagnostic flags change only what the run prints. `--dump-at-pc`
/// ends the run at its break.
fn run_retargets_anchor(args: &BootRunArgs, checkpoint_elsewhere: bool) -> bool {
    args.elf_path.is_some()
        || args.budget.is_some()
        || args.strict_reserved
        || !args.guest_arg.is_empty()
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

/// The cell this composition puts the run in.
///
/// Returns `None` when the composition names no key an anchor could be
/// filed under:
///
/// - an unmanaged or absent firmware carries no version;
/// - a title the store does not hold has no game-version axis;
/// - an executable outside the selected firmware entry belongs to no
///   entry.
pub(super) fn composed_cell(composition: &BootComposition) -> Option<CellKey> {
    let fw = composition.firmware.version()?.to_string();
    let game_ver = match &composition.game {
        GameChoice::Stored(stored) => Some(match &stored.version {
            GameVersion::Base => BASE_GAME_VER.to_string(),
            GameVersion::Update(v) => v.clone(),
        }),
        // The manifest named an absolute `firmware-exec` path, and the
        // composition keeps it as written. The executable therefore did
        // not come from the firmware entry this key would name.
        GameChoice::Firmware {
            unmanaged_path: true,
            ..
        } => return None,
        // A firmware-shipped executable has no version axis of its own,
        // so its cell is the firmware alone.
        GameChoice::Firmware { .. } => None,
        GameChoice::Unstored => return None,
    };
    Some(CellKey { fw, game_ver })
}

/// What the registry declares for the cell this run composed.
///
/// The run and the cell's anchor use the same cap and the same
/// checkpoint.
pub(super) struct ResolvedPlan {
    pub(super) cell: Option<CellKey>,
    max_steps: u64,
    checkpoint: game::manifest::CheckpointTrigger,
}

impl ResolvedPlan {
    pub(super) fn resolve(
        title: &game::manifest::TitleManifest,
        composition: &BootComposition,
    ) -> Self {
        let cell = composed_cell(composition);
        // A cell the matrix does not declare takes the title's own
        // defaults.
        let declared = cell.as_ref().and_then(|k| title.cell(k));
        Self {
            max_steps: cell_max_steps(title, declared),
            checkpoint: cell_checkpoint(title, declared),
            cell,
        }
    }

    pub(super) fn as_plan(&self) -> game::AnchorPlan<'_> {
        game::AnchorPlan {
            cell: self.cell.as_ref(),
            max_steps: self.max_steps,
            checkpoint: self.checkpoint,
        }
    }

    /// The cap as a step count, so an un-overridden bench run stays
    /// comparable to the anchor `dev record-anchors` measured.
    pub(super) fn max_steps_usize(&self, title: &game::manifest::TitleManifest) -> usize {
        usize::try_from(self.max_steps).unwrap_or_else(|_| {
            die(&format!(
                "{}: bench_max_steps {} does not fit this host's usize",
                title.name(),
                self.max_steps
            ))
        })
    }
}

pub(crate) fn bench_boot_once(args: &BenchArgs, vfs_flag: Option<&Path>, render: RenderFlags) {
    let vfs_root = resolve_ps3_vfs_root(vfs_flag);
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        &vfs_root,
        None,
        "boot bench-once",
    );
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    let max_steps = args
        .max_steps
        .unwrap_or_else(|| plan.max_steps_usize(&inputs.title));
    let firmware_dir = firmware_module_dir(&inputs.composition);
    let selection = selection_args(&args.selection, vfs_flag);
    let bar = ProgressBar::start(render.caps(), &BENCH_TASK, inputs.title.name());
    let sink = bar.sink();
    game::bench_boot_one_run(
        game::BenchOptions {
            title: &inputs.title,
            elf_path: &inputs.elf_path,
            max_steps,
            plan: plan.as_plan(),
            firmware_dir: firmware_dir.as_deref(),
            composed_mounts: &inputs.composition.mounts,
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
    bar.finish();
}

/// Both `bench` and `bench-once` flatten [`BenchArgs`], so clap accepts
/// `--save-state-trace` and `--run-index` on either. The run set
/// forwards neither to its children.
fn refuse_bench_once_only_flags(args: &BenchArgs) {
    if let Some(path) = &args.save_state_trace {
        die_usage(&format!(
            "boot bench: --save-state-trace {path} names one path, and a run set takes \
             several measurements that would each write over it. A traced boot is a \
             divergence diagnostic rather than a measurement, so take it with \
             `boot bench-once --save-state-trace PATH`."
        ));
    }
    if let Some(index) = args.run_index {
        die_usage(&format!(
            "boot bench: --run-index {index} has no meaning for a run set: the set stamps \
             each child it spawns with that child's own index. Pass it to \
             `boot bench-once` only."
        ));
    }
}

/// One measurement spreads against nothing, so a strict gate over it
/// would report OK for a check that never runs.
fn refuse_strict_perf_without_a_spread(gate_args: &BenchGateArgs) {
    if gate_args.strict_perf && gate_args.runs < 2 {
        die_usage(
            "boot bench: --strict-perf enforces the cross-run spread, and --runs 1 \
             measures no spread to enforce. Take at least two runs, or drop \
             --strict-perf.",
        );
    }
}

pub(crate) fn bench_boot(gate_args: &BenchGateArgs, vfs_flag: Option<&Path>, render: RenderFlags) {
    let args: &BenchArgs = &gate_args.bench;
    // Ahead of every resolution below: a refused invocation must not
    // first read the store.
    refuse_bench_once_only_flags(args);
    refuse_strict_perf_without_a_spread(gate_args);
    if gate_args.all {
        super::bench_all::run(gate_args, vfs_flag, render);
    }
    let vfs_root = resolve_ps3_vfs_root(vfs_flag);
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        &vfs_root,
        None,
        "boot bench",
    );
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    let max_steps = args
        .max_steps
        .unwrap_or_else(|| plan.max_steps_usize(&inputs.title));
    let firmware_dir = firmware_module_dir(&inputs.composition);
    let selection = selection_args(&args.selection, vfs_flag);
    let bar = ProgressBar::start(render.caps(), &BENCH_PAIR_TASK, inputs.title.name());
    let sink = bar.sink();
    let outcome = match game::bench_boot_runs(
        game::BenchOptions {
            title: &inputs.title,
            elf_path: &inputs.elf_path,
            max_steps,
            plan: plan.as_plan(),
            firmware_dir: firmware_dir.as_deref(),
            composed_mounts: &inputs.composition.mounts,
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
        Err(e) => {
            bar.abort();
            eprintln!("boot bench: {e}");
            let captured_stdout = e.captured_stdout();
            if !captured_stdout.is_empty() {
                eprintln!("stdout:\n{captured_stdout}");
            }
            let captured_stderr = e.captured_stderr();
            if !captured_stderr.is_empty() {
                eprintln!("stderr:\n{captured_stderr}");
            }
            std::process::exit(EXIT_SUBPROCESS_FAIL);
        }
    };
    // Down before the gate: every arm below it exits the process, and
    // `process::exit` runs no destructor.
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
            std::process::exit(EXIT_DETERMINISM_BREAK);
        }
        game::BenchGate::AnchorDrift => {
            let game::AnchorVerdict::Drift(failures) = &outcome.anchor else {
                unreachable!("invariant: only a drift verdict reaches the anchor-drift gate")
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
            std::process::exit(EXIT_ANCHOR_DRIFT);
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
                game::ThroughputVerdict::Measured { .. } => unreachable!(
                    "invariant: a measured throughput verdict does not reach the strict gate"
                ),
            };
            eprintln!(
                "boot bench: --strict-perf: no throughput verdict -- {detail}. \
                 Without --strict-perf this reports and exits 0: elapsed time on a host \
                 running anything else measures the host. \
                 exiting with status {EXIT_SPREAD_EXCEEDED}"
            );
            std::process::exit(EXIT_SPREAD_EXCEEDED);
        }
    }
}

#[cfg(test)]
#[path = "tests/boot_cmd_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/composition_wiring_tests.rs"]
mod composition_wiring_tests;

#[cfg(test)]
#[path = "tests/boot_run_finish_line_tests.rs"]
mod boot_run_finish_line_tests;
