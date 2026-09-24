//! The inputs one boot resolves: the title, its composition and the loaded executable.

use std::path::Path;

use cellgov_compare::BootOverrides;

use crate::composition::BootComposition;

use crate::cli::exit::CommandError;
use crate::cli::parse::{BootSelection, TitleSelector};
use crate::cli::self_load::{LoadPpuImageError, LoadedPpuImage};
use crate::cli::title::resolve_title_manifest;

use super::compose::resolve_composition;

pub(in crate::cli) struct BootInputs {
    pub(in crate::cli) title: cellgov_boot::manifest::TitleManifest,
    /// What the store composed for this run: the firmware, the game
    /// version, and the guest tree the two produce.
    pub(in crate::cli) composition: BootComposition,
    pub(in crate::cli) elf_path: String,
    /// Pre-loaded plaintext ELF bytes from the loader (explicit
    /// path or candidate walk). Passed to `prepare()` so the
    /// decrypt happens exactly once.
    pub(in crate::cli) elf_data: Vec<u8>,
    /// Program authority id from the SELF identification header;
    /// `None` for raw-ELF inputs (boot serves the retail fallback).
    pub(in crate::cli) authority_id: Option<u64>,
    pub(in crate::cli) control_flags1: Option<u32>,
}

pub(in crate::cli) fn resolve_boot_inputs(
    selector: &TitleSelector,
    selection: &BootSelection,
    overrides: BootOverrides,
    vfs_root: &Path,
    explicit_elf: Option<&str>,
    subcmd: &str,
) -> Result<BootInputs, CommandError> {
    let title = resolve_title_manifest(selector, subcmd)?;
    let composition = resolve_composition(selection, vfs_root, &title, overrides)?;
    let (elf_path, image) = match explicit_elf {
        Some(p) => {
            let image = crate::cli::self_load::load_ppu_image_with_title(p, &title, vfs_root)?;
            (p.to_string(), image)
        }
        None => {
            let loaded = crate::cli::self_load::load_ppu_image_walk_candidates(
                &title,
                vfs_root,
                &composition.eboot_dirs,
            );
            let (image, path) = match loaded {
                Ok(loaded) => loaded,
                Err(LoadPpuImageError::NotInstalled(error)) => {
                    eprintln!(
                        "{} title={} ({})",
                        cellgov_compare::witnesses::TITLE_NOT_INSTALLED_SENTINEL,
                        title.name(),
                        error.marker_note()
                    );
                    return Err(CommandError::failed(format!("load ppu image: {error}")));
                }
                Err(LoadPpuImageError::Failed(error)) => return Err(error),
            };
            (forwardable_eboot_path(&path, subcmd)?, image)
        }
    };
    Ok(boot_inputs(title, composition, elf_path, image))
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
/// Returns an error when:
///
/// - the dump is absent;
/// - a candidate cannot load;
/// - the command cannot pass the EBOOT path to a child command.
pub(in crate::cli) fn try_resolve_cell_inputs(
    title: cellgov_boot::manifest::TitleManifest,
    composition: BootComposition,
    vfs_root: &Path,
    subcmd: &str,
) -> Result<BootInputs, LoadPpuImageError> {
    let (image, path) = crate::cli::self_load::load_ppu_image_walk_candidates(
        &title,
        vfs_root,
        &composition.eboot_dirs,
    )?;
    let elf_path = forwardable_eboot_path(&path, subcmd)?;
    Ok(boot_inputs(title, composition, elf_path, image))
}

/// A resolved EBOOT path as a child invocation spells it.
fn forwardable_eboot_path(path: &Path, subcmd: &str) -> Result<String, CommandError> {
    path.to_str().map(|s| s.replace('\\', "/")).ok_or_else(|| {
        CommandError::failed(format!(
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
    title: cellgov_boot::manifest::TitleManifest,
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
