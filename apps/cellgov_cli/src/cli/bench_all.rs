//! `boot bench --all`: every declared cell of every registry title,
//! one after another, each held against its committed anchor.
//!
//! The cells come from [`super::declared_cells`], the same list
//! `dev record-anchors --all` records. Cells run serially in registry
//! order: the report reads the same every time, and no two boots
//! contend for host memory.

use std::collections::BTreeSet;
use std::path::Path;

use cellgov_terminal::caps::RenderFlags;
use cellgov_terminal::progress::ProgressBar;
use cellgov_time::Budget;

use super::boot_cmd::{
    firmware_module_dir, selection_args, separate_spawn_command_error, try_resolve_cell_inputs,
    try_resolve_composition, CompositionResolutionError, ResolvedPlan, EXIT_SPREAD_EXCEEDED,
};
use super::declared_cells::{
    declared_cells, filter_declared, read_registry, refuse_undeclared, DeclaredCell,
};
use super::exit::{CommandError, CommandExitCode};
use super::exit_codes;
use super::parse::{BenchGateArgs, BootSelection};
use super::self_load::LoadPpuImageError;
use super::title::{resolve_ps3_vfs_root, DEFAULT_TITLE_REGISTRY_DIR};
use crate::composition::{ComposeError, FirmwareSelectError, GameVersionSelectError};
use crate::game;
use crate::progress::BENCH_PAIR_TASK;
use cellgov_boot::manifest::TitleManifest;

/// How a refusal and the report name this invocation.
const COMMAND: &str = "boot bench --all";

/// The verdict a sweep gives one declared cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CellVerdict {
    /// The run reproduced the committed anchor.
    Matches,
    /// The run disagreed with the committed anchor; each string names
    /// one disagreement.
    Moved(Vec<String>),
    /// The registry declares the cell, and the anchor tree holds no
    /// anchor for it.
    NotRecorded,
    /// A retargeting flag made the run incomparable; each string names
    /// one.
    NotCompared(Vec<String>),
    /// `--no-anchor-check`.
    NotChecked,
    /// The firmware or the dump the cell names is not on this machine.
    NotInstalled(String),
    /// The registry's reason the cell has no measurement yet.
    Pending(String),
    /// The runs of the set disagreed with each other, this many ways.
    DeterminismBreak(usize),
    /// `--strict-perf` and the set reached no throughput verdict.
    NoThroughputVerdict,
    /// A measurement's subprocess failed.
    BootFailed(String),
}

impl CellVerdict {
    /// The status the sweep exits with when this is its worst cell.
    fn exit_code(&self) -> i32 {
        match self {
            Self::Matches
            | Self::NotCompared(_)
            | Self::NotChecked
            | Self::NotInstalled(_)
            | Self::Pending(_) => 0,
            Self::NotRecorded => exit_codes::FAILED,
            Self::NoThroughputVerdict => EXIT_SPREAD_EXCEEDED,
            Self::BootFailed(_) => exit_codes::DIVERGED,
            Self::Moved(_) => exit_codes::ANCHOR_MOVED,
            Self::DeterminismBreak(_) => exit_codes::DISAGREED,
        }
    }

    /// Which verdict the sweep's status reports when cells disagree;
    /// higher wins.
    ///
    /// A determinism break makes a witness stream meaningless, and a
    /// moved anchor is the finding the sweep exists for. Both outrank
    /// a boot that failed for a reason of its own.
    fn severity(&self) -> u8 {
        match self {
            Self::Matches
            | Self::NotCompared(_)
            | Self::NotChecked
            | Self::NotInstalled(_)
            | Self::Pending(_) => 0,
            Self::NotRecorded => 1,
            Self::NoThroughputVerdict => 2,
            Self::BootFailed(_) => 3,
            Self::Moved(_) => 4,
            Self::DeterminismBreak(_) => 5,
        }
    }

    /// Whether a boot ran for this cell.
    fn ran(&self) -> bool {
        !matches!(self, Self::NotInstalled(_) | Self::Pending(_))
    }

    /// The word the tally counts this verdict under.
    fn tally_word(&self) -> &'static str {
        match self {
            Self::Matches => "matched",
            Self::Moved(_) => "moved",
            Self::NotRecorded => "not recorded",
            Self::NotCompared(_) => "not compared",
            Self::NotChecked => "not checked",
            Self::NotInstalled(_) => "not installed",
            Self::Pending(_) => "pending",
            Self::DeterminismBreak(_) => "broke determinism",
            Self::NoThroughputVerdict => "without a throughput verdict",
            Self::BootFailed(_) => "failed to boot",
        }
    }
}

/// The status a sweep over `verdicts` exits with: its worst cell's.
pub(super) fn sweep_exit_code(verdicts: &[CellVerdict]) -> i32 {
    verdicts
        .iter()
        .max_by_key(|v| v.severity())
        .map_or(0, CellVerdict::exit_code)
}

/// The one line a cell's verdict prints, and the lines that detail it.
pub(super) fn summary_lines(cell: &DeclaredCell, verdict: &CellVerdict) -> Vec<String> {
    let label = cell.label();
    let mut lines = Vec::new();
    match verdict {
        CellVerdict::Matches => lines.push(format!("{label}: matches")),
        CellVerdict::Moved(failures) => {
            lines.push(format!(
                "{label}: moved ({} disagreement(s))",
                failures.len()
            ));
            lines.extend(failures.iter().map(|f| format!("    {f}")));
        }
        CellVerdict::NotRecorded => lines.push(format!(
            "{label}: not recorded -- the registry declares the cell and nothing gates it; \
             record it with `dev record-anchors --title {}`",
            cell.short_name
        )),
        CellVerdict::NotCompared(reasons) => {
            lines.push(format!("{label}: not compared ({})", reasons.join("; ")));
        }
        CellVerdict::NotChecked => {
            lines.push(format!("{label}: not checked (--no-anchor-check)"));
        }
        CellVerdict::NotInstalled(why) => {
            // A missing-candidate reason lists one line per candidate;
            // the continuation lines sit under the summary line.
            let mut parts = why.lines();
            let first = parts.next().unwrap_or_default();
            let mut rest: Vec<String> = parts.map(|l| format!("    {}", l.trim_start())).collect();
            match rest.last_mut() {
                None => lines.push(format!("{label}: not installed ({first})")),
                Some(last) => {
                    last.push(')');
                    lines.push(format!("{label}: not installed ({first}"));
                }
            }
            lines.extend(rest);
        }
        CellVerdict::Pending(reason) => lines.push(format!("{label}: pending ({reason})")),
        CellVerdict::DeterminismBreak(count) => lines.push(format!(
            "{label}: determinism break ({count} disagreement(s) across the set; the report \
             above names them)"
        )),
        CellVerdict::NoThroughputVerdict => {
            lines.push(format!(
                "{label}: no throughput verdict under --strict-perf"
            ));
        }
        CellVerdict::BootFailed(why) => lines.push(format!("{label}: boot failed ({why})")),
    }
    lines
}

/// How many titles `cells` span; `--fw` / `--game-ver` can narrow a
/// sweep to fewer titles than the registry holds.
pub(super) fn titles_spanned(cells: &[DeclaredCell]) -> usize {
    cells
        .iter()
        .map(|c| c.content_id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

pub(super) fn nothing_ran_line(verdicts: &[CellVerdict]) -> String {
    let not_installed = verdicts
        .iter()
        .filter(|v| matches!(v, CellVerdict::NotInstalled(_)))
        .count();
    let pending = verdicts
        .iter()
        .filter(|v| matches!(v, CellVerdict::Pending(_)))
        .count();
    format!(
        "{COMMAND}: none of the {} declared cell(s) ran ({not_installed} not installed, \
         {pending} pending); nothing gated",
        verdicts.len()
    )
}

/// The tally line: every verdict kind the sweep saw, with its count,
/// in first-seen order.
pub(super) fn tally_line(verdicts: &[CellVerdict]) -> String {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for v in verdicts {
        let word = v.tally_word();
        match counts.iter_mut().find(|(w, _)| *w == word) {
            Some((_, n)) => *n += 1,
            None => counts.push((word, 1)),
        }
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(word, n)| format!("{n} {word}"))
        .collect();
    format!(
        "{COMMAND}: {} declared cell(s): {}",
        verdicts.len(),
        parts.join(", ")
    )
}

/// The half of the cell the store does not hold, or `None` for a
/// refusal that is not an absence.
///
/// The sweep reports an absence by name and gates nothing, as it does
/// for a dump the image walk cannot find. An update archived over an
/// uninstalled base is an absence: `status` reports that version as
/// not installed for the same reason (`game_version_is_installed` in
/// `cli::store::read::collect`). Every other refusal is a store error.
pub(super) fn not_installed_reason(e: &ComposeError) -> Option<String> {
    match e {
        ComposeError::Firmware(
            FirmwareSelectError::NotInstalled { .. } | FirmwareSelectError::NoneInstalled { .. },
        )
        | ComposeError::GameVersion(
            GameVersionSelectError::NotInstalled { .. }
            | GameVersionSelectError::OrphanUpdates { .. },
        )
        | ComposeError::TitleNotInStore { .. }
        | ComposeError::BaseRecordMissing { .. } => Some(e.to_string()),
        ComposeError::Firmware(_)
        | ComposeError::GameVersion(_)
        | ComposeError::Inventory(_)
        | ComposeError::FirmwareDirectory { .. }
        | ComposeError::IdentityRender { .. }
        | ComposeError::Identity(_)
        | ComposeError::ResolveEboot(_)
        | ComposeError::FirmwareRelativeWithoutEntry { .. }
        | ComposeError::TreeMissing { .. }
        | ComposeError::TreeUnreadable { .. }
        | ComposeError::ReadExdata { .. }
        | ComposeError::ExdataConflict { .. } => None,
    }
}

/// The verdict a completed run set gives its cell.
pub(super) fn classify(outcome: game::BenchRunsOutcome) -> Result<CellVerdict, CommandError> {
    let verdict = match outcome.gate {
        game::BenchGate::DeterminismBreak => {
            CellVerdict::DeterminismBreak(outcome.determinism_failures.len())
        }
        game::BenchGate::AnchorDrift => match outcome.anchor {
            game::AnchorVerdict::Drift(failures) => CellVerdict::Moved(failures),
            _ => {
                return Err(CommandError::failed(
                    "boot bench --all: anchor-drift gate carried no drift verdict",
                ))
            }
        },
        game::BenchGate::SpreadExceeded => CellVerdict::NoThroughputVerdict,
        game::BenchGate::Pass => match outcome.anchor {
            game::AnchorVerdict::Match => CellVerdict::Matches,
            game::AnchorVerdict::NotRecorded(_) => CellVerdict::NotRecorded,
            game::AnchorVerdict::NotComparable(reasons) => CellVerdict::NotCompared(reasons),
            game::AnchorVerdict::Skipped => CellVerdict::NotChecked,
            game::AnchorVerdict::Drift(_) => {
                return Err(CommandError::failed(
                    "boot bench --all: passing gate carried an anchor drift",
                ))
            }
        },
    };
    Ok(verdict)
}

/// Boot one declared cell as a run set and hold it against its anchor.
///
/// The sweep composes the cell by its own firmware and game version,
/// so the child runs re-resolve the same cell. It checks the composed
/// key against the declared one before it measures anything.
fn gate_cell(
    cell: &DeclaredCell,
    title: &TitleManifest,
    gate_args: &BenchGateArgs,
    vfs_flag: Option<&Path>,
    vfs_root: &Path,
    render: RenderFlags,
) -> Result<CellVerdict, CommandError> {
    let args = &gate_args.bench;
    let selection = BootSelection {
        fw: Some(cell.cell.fw.clone()),
        game_ver: cell.cell.game_ver.clone(),
        firmware_dir: None,
    };
    let composition =
        match try_resolve_composition(&selection, vfs_root, title, args.overrides.overrides()) {
            Ok(c) => c,
            Err(CompositionResolutionError::Compose(e)) => match not_installed_reason(&e) {
                Some(why) => return Ok(CellVerdict::NotInstalled(why)),
                None => {
                    return Err(CommandError::failed(format!(
                        "{COMMAND}: {}: {e}",
                        cell.label()
                    )))
                }
            },
            Err(CompositionResolutionError::Command(error)) => return Err(error),
        };
    let inputs = match try_resolve_cell_inputs(title.clone(), composition, vfs_root, COMMAND) {
        Ok(i) => i,
        Err(LoadPpuImageError::NotInstalled(error)) => {
            return Ok(CellVerdict::NotInstalled(error.to_string()))
        }
        Err(LoadPpuImageError::Failed(error)) => return Err(error),
    };
    let plan = ResolvedPlan::resolve(&inputs.title, &inputs.composition);
    if plan.cell.as_ref() != Some(&cell.cell) {
        return Err(CommandError::failed(format!(
            "{COMMAND}: {}: the composition keyed the run under {} rather than the declared \
             cell; refusing to hold one cell's run against another cell's anchor",
            cell.label(),
            plan.cell
                .as_ref()
                .map_or_else(|| "no cell".to_string(), |c| c.label()),
        )));
    }
    let max_steps = match args.max_steps {
        Some(max_steps) => max_steps,
        None => plan.max_steps_usize(&inputs.title)?,
    };
    let firmware_dir = firmware_module_dir(&inputs.composition)?;
    let owned = selection_args(&selection, vfs_flag)?;
    let bar = ProgressBar::start(render.caps(), &BENCH_PAIR_TASK, inputs.title.name());
    let sink = bar.sink();
    let outcome = game::bench_boot_runs(
        game::BenchOptions {
            title: &inputs.title,
            elf_path: &inputs.elf_path,
            max_steps,
            plan: plan.as_plan(),
            firmware_dir: firmware_dir.as_deref(),
            composed_mounts: &inputs.composition.mounts,
            eboot_dirs: &inputs.composition.eboot_dirs,
            identity: &inputs.composition.identity,
            selection: owned.as_args(),
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
    );
    match outcome {
        Ok(o) => {
            bar.finish();
            classify(o)
        }
        Err(game::SpawnError::Command(error)) => {
            bar.abort();
            Err(error)
        }
        Err(error) => {
            bar.abort();
            let e = separate_spawn_command_error(error)?;
            let stderr_tail: Vec<&str> = e.captured_stderr().lines().rev().take(8).collect();
            if !stderr_tail.is_empty() {
                eprintln!("{COMMAND}: {}: stderr tail:", cell.label());
                for line in stderr_tail.into_iter().rev() {
                    eprintln!("  {line}");
                }
            }
            Ok(CellVerdict::BootFailed(e.to_string()))
        }
    }
}

/// Refuse a sweep over an unmanaged firmware tree.
fn refuse_firmware_dir(gate_args: &BenchGateArgs) -> Result<(), CommandError> {
    if let Some(dir) = &gate_args.bench.selection.firmware_dir {
        return Err(CommandError::status(
            exit_codes::USAGE,
            format!(
                "{COMMAND}: --firmware-dir {} names an unmanaged tree, which carries no firmware \
             version, and a sweep names each cell by the version its row declares. Gate one \
             cell against that tree with `boot bench --title NAME --firmware-dir DIR`.",
                dir.display()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn run(
    gate_args: &BenchGateArgs,
    vfs_flag: Option<&Path>,
    render: RenderFlags,
) -> Result<CommandExitCode, CommandError> {
    // A refused invocation must not read the store, so this runs ahead
    // of every resolution below.
    refuse_firmware_dir(gate_args)?;
    let selection = &gate_args.bench.selection;
    // The sweep reads the registry the children resolve `--title`
    // against, so a child composes the manifest the sweep enumerated.
    let titles = read_registry(Path::new(DEFAULT_TITLE_REGISTRY_DIR))?;
    if titles.is_empty() {
        return Err(CommandError::failed(format!(
            "{COMMAND}: no title manifests under {DEFAULT_TITLE_REGISTRY_DIR}"
        )));
    }
    let selected: Vec<&TitleManifest> = titles.iter().collect();
    refuse_undeclared(&selected)?;
    let cells = filter_declared(
        selected.into_iter().flat_map(declared_cells).collect(),
        selection.fw.as_deref(),
        selection.game_ver.as_deref(),
        COMMAND,
    )?;
    let vfs_root = resolve_ps3_vfs_root(vfs_flag)?;
    println!(
        "{COMMAND}: {} declared cell(s) over {} title(s), runs={} per cell",
        cells.len(),
        titles_spanned(&cells),
        gate_args.runs,
    );

    let mut verdicts: Vec<CellVerdict> = Vec::with_capacity(cells.len());
    for cell in &cells {
        // The sweep sets a pending cell aside whatever `--fw` /
        // `--game-ver` named: the gate never measures what the registry
        // says it cannot, and `boot bench --title` tries one cell
        // regardless.
        let verdict = match &cell.pending {
            Some(reason) => CellVerdict::Pending(reason.clone()),
            None => {
                let title = titles
                    .iter()
                    .find(|t| t.content_id == cell.content_id)
                    .ok_or_else(|| {
                        CommandError::failed(format!(
                            "{COMMAND}: declared cell {} has no registry title",
                            cell.label()
                        ))
                    })?;
                gate_cell(cell, title, gate_args, vfs_flag, &vfs_root, render)?
            }
        };
        for line in summary_lines(cell, &verdict) {
            println!("{line}");
        }
        verdicts.push(verdict);
    }

    println!("{}", tally_line(&verdicts));
    // A sweep over a machine that holds none of its cells gated
    // nothing, and must not exit 0.
    if !verdicts.iter().any(CellVerdict::ran) {
        return Err(CommandError::failed(nothing_ran_line(&verdicts)));
    }
    let code = sweep_exit_code(&verdicts);
    if code != 0 {
        println!("{COMMAND}: exiting with status {code}");
    }
    Ok(CommandExitCode::new(code))
}

#[cfg(test)]
#[path = "tests/bench_all_tests.rs"]
mod tests;
