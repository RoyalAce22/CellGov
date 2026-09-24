//! Composition resolution for the boot family, the selection a child run is given, and the cell plan as the commands take it.

use std::path::Path;

use cellgov_compare::BootOverrides;

use crate::composition::{banner, compose_boot, BootComposition, ComposeError, ComposeInputs};
use crate::game;
use cellgov_boot::compose::ResolvedPlan;

use crate::cli::env::parse_env_bool;
use crate::cli::exit::CommandError;
use crate::cli::parse::BootSelection;

/// Set to `1` by synthetic harnesses (e.g. ps3autotests) to suppress
/// the auto-default.
pub(crate) const DISABLE_DEFAULT_ENV: &str = crate::env_vars::NO_FIRMWARE_DIR;

#[derive(Debug, thiserror::Error)]
pub(in crate::cli) enum CompositionResolutionError {
    #[error(transparent)]
    Compose(#[from] ComposeError),
    #[error(transparent)]
    Command(#[from] CommandError),
}

impl CompositionResolutionError {
    pub(super) fn into_boot_command_error(self) -> CommandError {
        match self {
            Self::Compose(error) => CommandError::failed(format!("boot: {error}")),
            Self::Command(error) => error,
        }
    }
}

/// Resolve `--fw`, `--game-ver` and `--firmware-dir` against the
/// store, then print the selection banner before any other output.
///
/// An unresolved selection is fatal here. A title that boots without
/// firmware binds no import and dies dozens of steps later naming a
/// NID, which says nothing about the firmware.
pub(in crate::cli) fn resolve_composition(
    selection: &BootSelection,
    vfs_root: &Path,
    title: &cellgov_boot::manifest::TitleManifest,
    overrides: BootOverrides,
) -> Result<BootComposition, CommandError> {
    let composition = try_resolve_composition(selection, vfs_root, title, overrides)
        .map_err(CompositionResolutionError::into_boot_command_error)?;
    Ok(composition)
}

/// [`resolve_composition`] with the refusal returned, so a sweep can
/// name the cell it stops and continue to the next.
///
/// The composed identity takes `overrides` before the identity line
/// prints, so the line names every override the boot applies.
///
/// # Errors
///
/// Returns an error when:
///
/// - composition fails;
/// - command input is invalid.
///
/// The banner prints only after composition succeeds.
pub(in crate::cli) fn try_resolve_composition(
    selection: &BootSelection,
    vfs_root: &Path,
    title: &cellgov_boot::manifest::TitleManifest,
    overrides: BootOverrides,
) -> Result<BootComposition, CompositionResolutionError> {
    if let Some(explicit) = &selection.firmware_dir {
        if !explicit.is_dir() {
            return Err(ComposeError::FirmwareDirectory {
                path: explicit.display().to_string(),
            }
            .into());
        }
    }
    let install_root = crate::cli::keys::install_root_of(vfs_root);
    let mut composition = compose_boot(&ComposeInputs {
        title,
        vfs_root,
        install_root: &install_root,
        fw: selection.fw.as_deref(),
        game_ver: selection.game_ver.as_deref(),
        firmware_dir: selection.firmware_dir.as_deref(),
        // The value decides: `CELLGOV_NO_FIRMWARE_DIR=0` leaves the
        // default in place.
        no_firmware: parse_env_bool(DISABLE_DEFAULT_ENV)?,
    })
    .map_err(ComposeError::from)?;
    composition.identity.overrides = overrides;
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
        Err(error) => {
            return Err(ComposeError::IdentityRender {
                message: error.to_string(),
            }
            .into())
        }
    }
    Ok(composition)
}

/// [`cellgov_boot::compose::firmware_module_dir`] as the string the
/// firmware loader takes.
///
/// # Errors
///
/// Returns an error if the module path is not valid UTF-8.
pub(in crate::cli) fn firmware_module_dir(
    composition: &BootComposition,
) -> Result<Option<String>, CommandError> {
    let Some(dir) = cellgov_boot::compose::firmware_module_dir(composition) else {
        return Ok(None);
    };
    let dir = dir.to_str().ok_or_else(|| {
        CommandError::failed(format!(
            "boot: firmware module directory {} is not valid UTF-8",
            dir.display()
        ))
    })?;
    Ok(Some(dir.to_string()))
}

/// The selection flags this process received, owned so a
/// [`game::SelectionArgs`] can borrow them across the call that
/// encodes a child invocation.
pub(in crate::cli) struct OwnedSelection {
    fw: Option<String>,
    game_ver: Option<String>,
    firmware_dir: Option<String>,
    vfs_root: Option<String>,
}

impl OwnedSelection {
    pub(in crate::cli) fn as_args(&self) -> game::SelectionArgs<'_> {
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
fn forwardable(path: Option<&Path>, flag: &str) -> Result<Option<String>, CommandError> {
    path.map(|p| {
        p.to_str().map(str::to_string).ok_or_else(|| {
            CommandError::failed(format!(
                "{flag} {} is not valid UTF-8, so a child run cannot be given it",
                p.display()
            ))
        })
    })
    .transpose()
}

pub(in crate::cli) fn selection_args(
    selection: &BootSelection,
    vfs_flag: Option<&Path>,
) -> Result<OwnedSelection, CommandError> {
    Ok(OwnedSelection {
        fw: selection.fw.clone(),
        game_ver: selection.game_ver.clone(),
        firmware_dir: forwardable(selection.firmware_dir.as_deref(), "--firmware-dir")?,
        vfs_root: forwardable(vfs_flag, "--vfs-root")?,
    })
}

/// The plan as the bench and run paths take it.
pub(in crate::cli) fn anchor_plan(plan: &ResolvedPlan) -> game::AnchorPlan<'_> {
    game::AnchorPlan {
        cell: plan.cell.as_ref(),
        max_steps: plan.max_steps,
        checkpoint: plan.checkpoint,
    }
}

/// The cap as a step count, so an un-overridden bench run stays
/// comparable to the anchor `dev record-anchors` measured.
///
/// # Errors
///
/// Returns an error if the cap does not fit `usize`.
pub(in crate::cli) fn plan_max_steps(
    plan: &ResolvedPlan,
    title: &cellgov_boot::manifest::TitleManifest,
) -> Result<usize, CommandError> {
    plan.max_steps_usize().ok_or_else(|| {
        CommandError::failed(format!(
            "{}: bench_max_steps {} does not fit this host's usize",
            title.name(),
            plan.max_steps
        ))
    })
}

#[cfg(test)]
#[path = "tests/composition_wiring_tests.rs"]
mod composition_wiring_tests;
