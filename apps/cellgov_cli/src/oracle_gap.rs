//! Builds the operator-local oracle dispatch-gap overlay.
//!
//! `cellgov_lv2_archive` parses the dispatch table and owns the
//! overlay's text form; this command finds the checkout, reads its
//! revision and writes the file.

use std::path::{Path, PathBuf};

use cellgov_lv2_archive as archive;

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::paths::workspace_root;

/// Where the overlay lives for a PS3 VFS root: in the operator metadata
/// directory of the install root above it, beside the key vault.
/// `dev fixture-gen` reads it from the same place.
pub(crate) fn overlay_path(vfs_root: &Path) -> PathBuf {
    crate::cli::keys::install_root_of(vfs_root).join(".cellgov/oracle-gap.tsv")
}

/// Writes the source revision and each unbound table slot.
///
/// # Errors
///
/// Returns an error if:
///
/// - The command cannot resolve the VFS root.
/// - Git cannot read the checkout revision.
/// - The command cannot read the dispatch table.
/// - The command cannot write the overlay.
pub(crate) fn run(vfs_flag: Option<&Path>) -> Result<CommandExitCode, CommandError> {
    let root = crate::cli::title::resolve_ps3_vfs_root(vfs_flag)?;
    let checkout = ["rpc", "s3-src"].concat();
    let checkout_root = workspace_root().join("tools").join(checkout);
    let source = checkout_root.join(["rpc", "s3/Emu/Cell/lv2/lv2.cpp"].concat());
    if !source.exists() {
        println!("oracle gap: not computed -- local oracle checkout is unavailable");
        return Ok(CommandExitCode::SUCCESS);
    }
    let revision = std::process::Command::new("git")
        .arg("-C")
        .arg(&checkout_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| {
            CommandError::failed(format!("oracle gap: read checkout revision: {error}"))
        })?;
    if !revision.status.success() {
        return Err(CommandError::failed(
            "oracle gap: checkout has no readable revision",
        ));
    }
    let table = std::fs::read_to_string(&source).map_err(|error| {
        CommandError::failed(format!("oracle gap: read dispatch table: {error}"))
    })?;
    let out = overlay_path(&root);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CommandError::failed(format!("oracle gap: create overlay: {error}"))
        })?;
    }
    let text = archive::overlay_text(
        String::from_utf8_lossy(&revision.stdout).trim(),
        &archive::unbound_ordinals(&table),
    );
    std::fs::write(&out, text)
        .map_err(|error| CommandError::failed(format!("oracle gap: write overlay: {error}")))?;
    println!("oracle gap: wrote {}", out.display());
    Ok(CommandExitCode::SUCCESS)
}

#[cfg(test)]
#[path = "tests/oracle_gap_tests.rs"]
mod tests;
