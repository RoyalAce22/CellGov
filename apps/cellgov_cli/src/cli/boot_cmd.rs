//! Dispatch for the boot-family subcommands: `boot run`,
//! `boot bench-once`, and `boot bench`.

use std::path::Path;

use cellgov_compare::BootOutcome;
use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressBar;
use cellgov_time::Budget;

use crate::composition::{banner, compose_boot, BootComposition, ComposeInputs, FirmwareChoice};
use crate::game;
use crate::progress::{BENCH_PAIR_TASK, BENCH_TASK, RUN_TASK};

use super::env::parse_env_bool;
use super::exit::die;
use super::parse::{BenchArgs, BootRunArgs, BootSelection, TitleSelector};
use super::title::{resolve_ps3_vfs_root, resolve_title_manifest};
use crate::paths::anchor_max_steps;

use game::BENCH_AGREEMENT_GATE_PCT;

/// The `sys/external` modules inside a firmware entry, relative to the
/// entry's `dev_flash` mount.
const FIRMWARE_EXTERNAL: [&str; 2] = ["sys", "external"];

/// Set to `1` by synthetic harnesses (e.g. ps3autotests) to suppress
/// the auto-default.
const DISABLE_DEFAULT_ENV: &str = "CELLGOV_NO_FIRMWARE_DIR";

/// Exit code: two bench runs disagreed on step count or outcome.
const EXIT_DETERMINISM_BREAK: i32 = 3;

/// Exit code: wall-time disagreement exceeded the gate or was
/// unmeasurable. The value sits above the shared 0-5 contract, which
/// gives 2 to a usage error `boot bench` can also return.
const EXIT_WALL_DRIFT: i32 = 15;

/// Exit code: a bench subprocess failed or its `BENCH_RESULT` line was
/// unparseable.
const EXIT_SUBPROCESS_FAIL: i32 = 4;

/// Exit code: the run disagreed with the title's committed anchor.
const EXIT_ANCHOR_DRIFT: i32 = 5;

/// `boot run` terminated with a guest fault.
const EXIT_RUN_GAME_FAULT: i32 = 10;
/// `boot run` reached `--max-steps` without hitting the configured
/// checkpoint.
const EXIT_RUN_GAME_MAX_STEPS: i32 = 11;
/// `boot run` exhausted simulated time before reaching a terminal
/// state.
const EXIT_RUN_GAME_TIME_OVERFLOW: i32 = 12;
/// `boot run` completed but the loop logged an anomaly that violates
/// the determinism contract (lost syscall-wake responses).
const EXIT_RUN_GAME_CRITICAL_ANOMALY: i32 = 13;
/// A `--save-observation` / `--save-boot-summary` artifact was
/// requested but writing its JSON failed.
const EXIT_RUN_GAME_SAVE_ARTIFACT: i32 = 14;

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
    })
    .unwrap_or_else(|e| die(&format!("boot: {e}")));
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
    composition
}

/// The `sys/external` directory the firmware loader reads its modules
/// from, or `None` for a boot with no firmware.
fn firmware_module_dir(composition: &BootComposition) -> Option<String> {
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
struct OwnedSelection {
    fw: Option<String>,
    game_ver: Option<String>,
    firmware_dir: Option<String>,
    vfs_root: Option<String>,
}

impl OwnedSelection {
    fn as_args(&self) -> game::SelectionArgs<'_> {
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
                    "{flag} {} is not valid UTF-8, so the paired run cannot be given it",
                    p.display()
                ))
            })
            .to_string()
    })
}

fn selection_args(selection: &BootSelection, vfs_flag: Option<&Path>) -> OwnedSelection {
    OwnedSelection {
        fw: selection.fw.clone(),
        game_ver: selection.game_ver.clone(),
        firmware_dir: forwardable(selection.firmware_dir.as_deref(), "--firmware-dir"),
        vfs_root: forwardable(vfs_flag, "--vfs-root"),
    }
}

struct BootInputs {
    title: game::manifest::TitleManifest,
    /// What the store composed for this run: the firmware, the game
    /// version, and the guest tree the two produce.
    composition: BootComposition,
    elf_path: String,
    /// Pre-loaded plaintext ELF bytes from the loader (explicit
    /// path or candidate walk). Passed to `prepare()` so the
    /// decrypt happens exactly once.
    elf_data: Vec<u8>,
    /// Program authority id from the SELF identification header;
    /// `None` for raw-ELF inputs (boot serves the retail fallback).
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
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
            let path_str = path
                .to_str()
                .map(|s| s.replace('\\', "/"))
                .unwrap_or_else(|| {
                    die(&format!(
                        "{subcmd}: resolved EBOOT path is not valid UTF-8: {}",
                        path.display()
                    ))
                });
            (path_str, image)
        }
    };
    // Past this line a failing run is a boot failure, never a missing
    // dump; the suites key their skip/fail split on it.
    //
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

/// [`anchor_max_steps`] as a step count, so an un-overridden bench run
/// stays comparable to the anchor `dev record-anchors` measured.
fn default_bench_max_steps(title: &game::manifest::TitleManifest) -> usize {
    let cap = anchor_max_steps(title);
    usize::try_from(cap).unwrap_or_else(|_| {
        die(&format!(
            "{}: bench_max_steps {cap} does not fit this host's usize",
            title.name()
        ))
    })
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
    let max_steps = args
        .max_steps
        .unwrap_or_else(|| default_bench_max_steps(&inputs.title));
    let firmware_dir = firmware_module_dir(&inputs.composition);
    let selection = selection_args(&args.selection, vfs_flag);
    let bar = ProgressBar::start(render.caps(), &BENCH_TASK, inputs.title.name());
    let sink = bar.sink();
    game::bench_boot_one_run(
        game::BenchOptions {
            title: &inputs.title,
            elf_path: &inputs.elf_path,
            max_steps,
            firmware_dir: firmware_dir.as_deref(),
            composed_mounts: &inputs.composition.mounts,
            identity: &inputs.composition.identity,
            selection: selection.as_args(),
            strict_reserved: args.strict_reserved,
            checkpoint_override: args.checkpoint,
            budget_override: args.budget.map(Budget::new),
            prescan: args.prescan,
            guest_args: &args.guest_arg,
            // This entry point is the raw measurement the pair spawns
            // twice; only the pair gates on the anchor.
            check_anchor: false,
        },
        inputs.elf_data,
        inputs.authority_id,
        inputs.control_flags1,
        &*sink,
    );
    bar.finish();
}

pub(crate) fn bench_boot(
    args: &BenchArgs,
    check_anchor: bool,
    vfs_flag: Option<&Path>,
    render: RenderFlags,
) {
    let vfs_root = resolve_ps3_vfs_root(vfs_flag);
    let inputs = resolve_boot_inputs(
        &args.selector,
        &args.selection,
        &vfs_root,
        None,
        "boot bench",
    );
    let max_steps = args
        .max_steps
        .unwrap_or_else(|| default_bench_max_steps(&inputs.title));
    let firmware_dir = firmware_module_dir(&inputs.composition);
    let selection = selection_args(&args.selection, vfs_flag);
    let bar = ProgressBar::start(render.caps(), &BENCH_PAIR_TASK, inputs.title.name());
    let sink = bar.sink();
    let outcome = match game::bench_boot_pair(
        game::BenchOptions {
            title: &inputs.title,
            elf_path: &inputs.elf_path,
            max_steps,
            firmware_dir: firmware_dir.as_deref(),
            composed_mounts: &inputs.composition.mounts,
            identity: &inputs.composition.identity,
            selection: selection.as_args(),
            strict_reserved: args.strict_reserved,
            checkpoint_override: args.checkpoint,
            budget_override: args.budget.map(Budget::new),
            prescan: args.prescan,
            guest_args: &args.guest_arg,
            check_anchor,
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
            // Steps and outcome can match here: a witness that moves
            // between runs is also a determinism break, and the pair
            // printed those disagreements to stdout above.
            eprintln!(
                "boot bench: determinism break: run 1 steps={} outcome={}, \
                 run 2 steps={} outcome={}. Identical steps and outcome here mean \
                 the runs disagreed on a witness; see the disagreements above. \
                 Exiting with status {EXIT_DETERMINISM_BREAK}",
                outcome.run1.steps, outcome.run1.outcome, outcome.run2.steps, outcome.run2.outcome,
            );
            std::process::exit(EXIT_DETERMINISM_BREAK);
        }
        game::BenchGate::AnchorDrift => {
            eprintln!(
                "boot bench: {} disagreement(s) with the committed anchor for {} \
                 (content id {}):",
                outcome.anchor_failures.len(),
                inputs.title.name(),
                inputs.title.content_id,
            );
            for failure in &outcome.anchor_failures {
                eprintln!("  {failure}");
            }
            eprintln!(
                "this run used the configuration the anchor was recorded under, so \
                 the movement is a regression until it is attributed to a change. \
                 Once it is, re-bless with:\n  \
                 cargo run --release -p cellgov_cli -- dev record-anchors --title {}\n\
                 --no-anchor-check drops this gate for a measurement-only run.\n\
                 exiting with status {EXIT_ANCHOR_DRIFT}",
                inputs.title.name(),
            );
            std::process::exit(EXIT_ANCHOR_DRIFT);
        }
        game::BenchGate::WallUnmeasurable => {
            eprintln!(
                "boot bench: wall measurement unusable (zero / non-finite); \
                 run 1 wall {:?}, run 2 wall {:?}; exiting with status {EXIT_WALL_DRIFT}",
                outcome.run1.wall, outcome.run2.wall
            );
            std::process::exit(EXIT_WALL_DRIFT);
        }
        game::BenchGate::WallDriftExceeded => {
            let drift = outcome.drift_pct.unwrap_or(f64::NAN);
            eprintln!(
                "boot bench: wall disagreement {drift:.2}% exceeds {BENCH_AGREEMENT_GATE_PCT:.1}% gate; \
                 exiting with status {EXIT_WALL_DRIFT}"
            );
            std::process::exit(EXIT_WALL_DRIFT);
        }
    }
}

#[cfg(test)]
#[path = "tests/boot_cmd_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/composition_wiring_tests.rs"]
mod composition_wiring_tests;
