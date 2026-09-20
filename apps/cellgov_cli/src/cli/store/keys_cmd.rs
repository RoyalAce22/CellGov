//! `cellgov keys show | import | remove`.

use std::path::{Path, PathBuf};

use cellgov_install::keys::{installed_keys_dir, KeyVault, SelfClass, Slot, INSTALLED_KEYS_FILE};

use crate::cli::parse::{KeysCommand, KeysPathArgs};

use super::StoreCliError;
use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;

/// Exit status of `keys show` when a decrypt path would find a key
/// missing.
const EXIT_KEYS_INCOMPLETE: i32 = exit_codes::command_specific(40);

/// Run one `keys` command against the vault under `store`.
pub(crate) fn run(command: &KeysCommand, store: &Path) -> Result<CommandExitCode, CommandError> {
    match command {
        KeysCommand::Show { path, .. } => show(path.as_deref(), store),
        KeysCommand::Import(args) => {
            import(args, store)?;
            Ok(CommandExitCode::SUCCESS)
        }
        KeysCommand::Remove { .. } => {
            remove(store)?;
            Ok(CommandExitCode::SUCCESS)
        }
    }
}

fn show(path: Option<&Path>, store: &Path) -> Result<CommandExitCode, CommandError> {
    let location = match path {
        Some(p) => p.to_path_buf(),
        None => KeyVault::locate_from(std::env::var_os(crate::env_vars::KEYS), store)
            .map_err(|error| CommandError::failed(error.to_string()))?,
    };
    let vault = KeyVault::load_from_path(&location)
        .map_err(|error| CommandError::failed(error.to_string()))?;
    print!("{}", render_inventory(&location, &vault));
    if !vault.missing_for_decrypt().is_empty() {
        return Ok(CommandExitCode::new(EXIT_KEYS_INCOMPLETE));
    }
    Ok(CommandExitCode::SUCCESS)
}

fn import(args: &KeysPathArgs, store: &Path) -> Result<(), CommandError> {
    let file = installed_keys_dir(store).join(INSTALLED_KEYS_FILE);
    // This check runs before the import writes: "merged" is true only
    // when a vault was already there to merge into.
    let merged = !args.replace && file.is_file();
    let vault = merge_into_installed(&args.path, store, args.replace)
        .map_err(|error| CommandError::failed(error.to_string()))?;
    println!(
        "cellgov: {} {} into {}",
        if merged { "merged" } else { "wrote" },
        args.path.display(),
        file.display()
    );
    println!("  {}", vault.summary());
    for ignored in vault.ignored() {
        println!("  set aside {}: {}", ignored.at, ignored.reason);
    }
    let missing = vault.missing_for_decrypt();
    if missing.is_empty() {
        println!("  decrypt paths: ready");
    } else {
        println!("  missing for decrypt: {}", missing.join(", "));
    }
    Ok(())
}

fn remove(store: &Path) -> Result<(), CommandError> {
    let dir = installed_keys_dir(store);
    match remove_installed(store) {
        Ok(true) => println!("cellgov: removed {}", dir.display()),
        Ok(false) => println!("cellgov: nothing installed at {}", dir.display()),
        Err(error) => return Err(CommandError::failed(error.to_string())),
    }
    Ok(())
}

/// The `keys show` report for the vault loaded from `location`.
fn render_inventory(location: &Path, vault: &KeyVault) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("key vault: {}", location.display()));
    lines.push(format!("  sources: {} file(s)", vault.sources().len()));
    for slot in Slot::ALL {
        lines.push(match vault.slot_provenance(slot) {
            Some(at) => format!("  {}: {} bytes, from {at}", slot.name(), slot.byte_len()),
            None => format!("  {}: missing", slot.name()),
        });
    }
    let scepkg = vault.scepkg_keys().map(Iterator::count).unwrap_or(0);
    lines.push(format!("  scepkg: {scepkg} keyset(s)"));
    for class in SelfClass::ALL {
        let labels = vault.labels(class);
        let labeled = if labels.is_empty() {
            "(none)".to_string()
        } else {
            labels.join(", ")
        };
        lines.push(format!(
            "  {class}: {} {labeled}, {} unlabeled",
            class.label_kind(),
            vault.unlabeled_count(class)
        ));
    }
    if !vault.ignored().is_empty() {
        lines.push("  set aside:".to_string());
        for ignored in vault.ignored() {
            lines.push(format!("    {}: {}", ignored.at, ignored.reason));
        }
    }
    let missing = vault.missing_for_decrypt();
    if missing.is_empty() {
        lines.push("  decrypt paths: ready".to_string());
    } else {
        lines.push(format!("  missing for decrypt: {}", missing.join(", ")));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Whether `vault` holds any value a decrypt path could use.
fn holds_any_key(vault: &KeyVault) -> bool {
    Slot::ALL
        .iter()
        .any(|s| vault.slot_provenance(*s).is_some())
        || vault.scepkg_keys().is_ok()
        || SelfClass::ALL.iter().any(|c| vault.keyset_count(*c) > 0)
}

/// Normalize the vault at `path` into `<store>/.cellgov/keys/keys.toml`.
///
/// Without `replace`, the import merges into the vault already there.
///
/// # Errors
///
/// - [`StoreCliError::Keys`] for a vault that will not load, or for a
///   merge whose two definitions of one key disagree. The refusal names
///   both definitions.
/// - [`StoreCliError::KeysNothingUsable`] when `path` held no key.
/// - [`StoreCliError::KeysDirCreateFailed`] and
///   [`StoreCliError::KeysWriteFailed`] for the write refusals.
fn merge_into_installed(
    path: &Path,
    store: &Path,
    replace: bool,
) -> Result<KeyVault, StoreCliError> {
    let imported = KeyVault::load_from_path(path)?;
    if !holds_any_key(&imported) {
        return Err(StoreCliError::KeysNothingUsable {
            path: path.to_path_buf(),
        });
    }
    let dir = installed_keys_dir(store);
    let file = dir.join(INSTALLED_KEYS_FILE);
    let vault = if !replace && file.is_file() {
        let mut existing = KeyVault::load_from_path(&file)?;
        existing.merge(imported)?;
        existing
    } else {
        imported
    };
    std::fs::create_dir_all(&dir).map_err(|source| StoreCliError::KeysDirCreateFailed {
        path: dir.clone(),
        source,
    })?;
    std::fs::write(&file, vault.to_toml()).map_err(|source| StoreCliError::KeysWriteFailed {
        path: file.clone(),
        source,
    })?;
    Ok(vault)
}

/// Delete `<store>/.cellgov/keys/`.
///
/// Returns `Ok(false)` when there was nothing to delete.
///
/// # Errors
///
/// [`StoreCliError::KeysRemoveFailed`] for any refusal other than
/// absence.
fn remove_installed(store: &Path) -> Result<bool, StoreCliError> {
    let dir: PathBuf = installed_keys_dir(store);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(StoreCliError::KeysRemoveFailed { path: dir, source }),
    }
}

#[cfg(test)]
#[path = "tests/keys_cmd_tests.rs"]
mod tests;
