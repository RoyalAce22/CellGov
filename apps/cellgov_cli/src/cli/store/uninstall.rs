//! `cellgov title uninstall` and `cellgov firmware uninstall`.
//!
//! Both resolve a plan first, print exactly what it would remove, and
//! only then confirm and run it. `--dry-run` stops after the plan.

use std::path::Path;

use cellgov_install::game_uninstall::UninstallScope;
use cellgov_install::{firmware_uninstall, game_uninstall};

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::{FirmwareUninstallArgs, UninstallArgs};
use cellgov_boot::manifest::{ManifestError, TitleRegistry};

use super::confirm::{confirm, Answers};
use super::registry_dir;

/// `cellgov title uninstall <TITLE_ID> [--ver V | --updates | --all]`
pub(crate) fn title(
    args: &UninstallArgs,
    store: &Path,
    answers: Answers,
) -> Result<CommandExitCode, CommandError> {
    let scope = args.scope();
    let plan = game_uninstall::plan(&args.title_id, store, &scope)
        .map_err(|error| CommandError::failed(format!("uninstall failed: {error}")))?;

    if plan.entries.is_empty() {
        // An empty plan means `--updates` found no update; every other
        // scope names an entry or refuses. Re-plan at Base scope to
        // tell that from a title with no record at all.
        if let Err(e) = game_uninstall::plan(&args.title_id, store, &UninstallScope::Base) {
            return Err(CommandError::failed(format!("uninstall failed: {e}")));
        }
        println!(
            "cellgov: title {} has no entry this scope names; nothing to remove",
            plan.title_id
        );
        return Ok(CommandExitCode::SUCCESS);
    }

    println!("cellgov: uninstall {} would remove", plan.title_id);
    for entry in &plan.entries {
        // The tree goes whole, so the recorded count is a floor on
        // what the removal takes.
        println!(
            "  {} -- {} removed whole, holding {} recorded file(s)",
            entry.version,
            entry.tree_dir.display(),
            entry.recorded_files
        );
        println!("     record {}", entry.record_path.display());
    }
    match (&plan.rap, args.keep_rap) {
        (Some(rap), false) => println!("  RAP {}", rap.display()),
        (Some(rap), true) => println!("  RAP {} (kept: --keep-rap)", rap.display()),
        (None, _) => {}
    }
    if !plan.kept_updates.is_empty() {
        println!("  keeping update(s) {}", plan.kept_updates.join(", "));
    }
    if args.dry_run {
        println!("  --dry-run: nothing was removed");
        return Ok(CommandExitCode::SUCCESS);
    }
    if !confirm(
        &format!(
            "remove {} entry/entries of {} whole, holding {} recorded file(s)?",
            plan.entries.len(),
            plan.title_id,
            plan.recorded_files()
        ),
        answers,
    ) {
        println!("cellgov: nothing was removed");
        return Ok(CommandExitCode::SUCCESS);
    }

    let opts = game_uninstall::UninstallOptions {
        verify: args.verify,
        keep_rap: args.keep_rap,
        force: args.force,
    };
    let outcome = game_uninstall::execute(&plan, opts)
        .map_err(|error| CommandError::failed(format!("uninstall failed: {error}")))?;

    println!("cellgov: uninstalled {}", outcome.title_id);
    for entry in &outcome.removed {
        println!(
            "  removed {} tree {}",
            entry.version,
            entry.tree_removed.display()
        );
        println!("  removed record {}", entry.record_removed.display());
    }
    if let Some(rap) = &outcome.rap_removed {
        println!("  removed RAP {}", rap.display());
    }
    if let Some(n) = outcome.files_verified {
        println!("  verified {n} files against the record before removal");
    }
    // Non-zero only under --force, the one way a divergence passes
    // the gate.
    if let Some(n) = outcome.files_diverged {
        if n > 0 {
            eprintln!("  --force overrode {n} recorded files that were missing or modified");
        }
    }
    super::report_rename_retries(outcome.rename_retries);
    Ok(CommandExitCode::SUCCESS)
}

/// `cellgov firmware uninstall <VERSION>`
pub(crate) fn firmware(
    args: &FirmwareUninstallArgs,
    store: &Path,
    answers: Answers,
) -> Result<CommandExitCode, CommandError> {
    let plan = firmware_uninstall::plan(&args.version, store)
        .map_err(|error| CommandError::failed(format!("firmware uninstall failed: {error}")))?;

    println!("cellgov: uninstall firmware {} would remove", plan.version);
    println!("  entry  {}", plan.entry_dir.display());
    println!("  record {}", plan.record_path.display());

    let anchored = match cells_anchored_on(&args.version) {
        Ok(cells) => cells,
        Err(e) => {
            eprintln!(
                "cellgov: the anchor gate did not run -- title registry: {e}; no committed anchor \
                 was checked against firmware {}",
                plan.version
            );
            Vec::new()
        }
    };
    if !anchored.is_empty() {
        println!(
            "  {} committed anchor(s) name this firmware: {}",
            anchored.len(),
            anchored.join(", ")
        );
    }

    // A dry run removes nothing, so it returns before the anchor
    // refusal below and reports the anchors as part of the plan.
    if args.dry_run {
        println!("  --dry-run: nothing was removed");
        return Ok(CommandExitCode::SUCCESS);
    }
    if !anchored.is_empty() {
        if !args.force {
            return Err(CommandError::status(
                crate::cli::exit_codes::USAGE,
                format!(
                "firmware {} is named by {} committed anchor(s) ({}); removing it turns the next \
                 `boot bench` of those cells into a resolve failure rather than a clear refusal. \
                 Pass --force to remove it anyway.",
                plan.version,
                anchored.len(),
                anchored.join(", "),
            ),
            ));
        }
        eprintln!(
            "  --force: {} committed anchor(s) name this firmware ({})",
            anchored.len(),
            anchored.join(", ")
        );
    }
    if args.verify {
        verify_before_removal(&plan, store, args.force)?;
    }
    if !confirm(
        &format!(
            "remove firmware {} (installed from PUP {})?",
            plan.version, plan.pup_sha256
        ),
        answers,
    ) {
        println!("cellgov: nothing was removed");
        return Ok(CommandExitCode::SUCCESS);
    }

    let outcome = firmware_uninstall::execute(&plan)
        .map_err(|error| CommandError::failed(format!("firmware uninstall failed: {error}")))?;
    println!("cellgov: uninstalled firmware {}", outcome.version);
    println!("  removed entry {}", outcome.entry_removed.display());
    println!("  removed record {}", outcome.record_removed.display());
    super::report_rename_retries(outcome.rename_retries);
    Ok(CommandExitCode::SUCCESS)
}

/// Checks the installed tree against its manifest before removal.
///
/// # Errors
///
/// Returns an error in these cases:
///
/// - Verification cannot run.
/// - The tree diverges and `force` is false.
fn verify_before_removal(
    plan: &firmware_uninstall::FirmwareUninstallPlan,
    store: &Path,
    force: bool,
) -> Result<(), CommandError> {
    let keys = cellgov_install::keys::KeyVault::load_for_vfs(store)
        .map_err(|error| CommandError::failed(format!("firmware uninstall --verify: {error}")))?;
    // An entry that stores no kernel is checked on its modules alone;
    // the removal takes the entry whatever the kernel's state.
    let report = cellgov_install::firmware_verify::verify_firmware_entry(
        &plan.entry_dir,
        plan.core_os.as_ref(),
        &keys,
    )
    .map_err(|error| CommandError::failed(format!("firmware uninstall --verify: {error}")))?
    .report;
    for fault in &report.divergences {
        eprintln!("  {fault}");
    }
    if report.is_clean() {
        println!(
            "  verified {} artefact(s) against firmware.toml and the record before removal",
            report.matched
        );
        return Ok(());
    }
    if !force {
        return Err(CommandError::failed(format!(
            "firmware {} diverged from its record in {} of {} artefact(s); pass --force to \
             remove it anyway",
            plan.version,
            report.divergences.len(),
            report.checked(),
        )));
    }
    eprintln!(
        "  --force overrode {} artefact(s) that diverged from the record",
        report.divergences.len()
    );
    Ok(())
}

/// Every committed anchor whose cell names firmware `version`, as
/// `<title> fw <V> x <game_ver>` labels.
///
/// # Errors
///
/// Returns [`ManifestError`] when the registry does not load. An empty
/// list is a different answer: no cell names the version.
fn cells_anchored_on(version: &str) -> Result<Vec<String>, ManifestError> {
    let root = crate::paths::workspace_root();
    let registry = TitleRegistry::scan_dir(&registry_dir())?;
    let mut out = Vec::new();
    for title in registry.iter() {
        for cell in &title.matrix {
            if cell.key.fw != version {
                continue;
            }
            if crate::paths::boot_anchor_path(&root, &title.content_id, &cell.key).is_file() {
                out.push(format!("{} {}", title.short_name, cell.key.label()));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/uninstall_tests.rs"]
mod tests;
