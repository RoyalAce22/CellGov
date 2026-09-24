//! SELF loading, decryption, and title-image selection.
//!
//! The RAP read, the SELF open and the EBOOT candidate walk live in
//! `cellgov_install` and `cellgov_boot::manifest`; this module words
//! their refusals and decides whether a SELF header that will not parse
//! refuses the image or falls back.

use std::path::{Path, PathBuf};

use cellgov_boot::manifest::{EbootLoadError, TitleManifest, TitleNotInstalled};
use cellgov_install::npdrm::{read_rap, NpdHeaderInfo, RapPresence};
use cellgov_install::sce::SceError;
use cellgov_install::self_image::{open_ppu_image, to_plaintext_elf, KeyPolicy, SelfIdentity};
use cellgov_install::store::hdd0_exdata_dir;

use super::exit::CommandError;

/// The note on SCE-wrapped input carried in the help of every command
/// that accepts such a path.
pub(crate) const SCE_INPUT_USAGE_NOTE: &str = if cfg!(feature = "decrypt") {
    "SCE-wrapped input:\n  \
     NPDRM EBOOTs resolve their RAP from <vfs-root>/home/00000001/exdata/,\n  \
     and the key vault from CELLGOV_KEYS, else <vfs-root>/../.cellgov/keys/."
} else {
    // `--vfs-root` is still parsed, and an empty value still refused
    // (`super::title::resolve_ps3_vfs_root`); only what it names is unread.
    "SCE-wrapped input:\n  \
     this build has no decrypt support: plaintext ELF / PRX only. An\n  \
     SCE-wrapped input is refused by name, and --vfs-root names no path\n  \
     this build reads; rebuild with --features decrypt to read one."
};

/// The decrypt-capability words a usage text may carry only in a build
/// that has the feature. `decrypt` itself is not one of them: the
/// feature-off note names it in its rebuild hint.
#[cfg(test)]
pub(crate) const DECRYPTION_CLAIMS: [&str; 3] = ["exdata", "RAP", "key vault"];

pub(crate) fn load_file(path: &str) -> Result<Vec<u8>, CommandError> {
    std::fs::read(path)
        .map_err(|error| CommandError::failed(format!("failed to read {path}: {error}")))
}

/// Decrypt one PPU image, resolving an NPDRM title's RAP from
/// `<content_id>.rap` under the exdata directory of `vfs_root`.
///
/// This function derives the RAP path rather than taking a name, so a
/// missing file is no RAP: a license-3 title falls back to the free
/// klicensee.
pub(crate) fn decrypt_ppu_self(
    bytes: &[u8],
    path: &str,
    vfs_root: &Path,
) -> Result<Vec<u8>, CommandError> {
    let exdata = hdd0_exdata_dir(vfs_root);
    let lookup = |npd: &NpdHeaderInfo| {
        read_rap(
            &exdata.join(format!("{}.rap", npd.content_id)),
            RapPresence::MayBeAbsent,
        )
    };
    match to_plaintext_elf(
        bytes,
        super::keys::key_vault_for(bytes)?,
        KeyPolicy::Auto(&lookup),
    ) {
        Ok(elf) => Ok(elf.into_owned()),
        Err(e @ SceError::NoRapForNpdrmTitle { .. }) => Err(CommandError::failed(format!(
            "{e}; expected its RAP at {}/<content_id>.rap",
            exdata.display()
        ))),
        Err(e) => Err(decrypt_refusal(path, &e)),
    }
}

/// The one line every vault refusal ends with.
const KEYS_HINT: &str = "supply keys with CELLGOV_KEYS=<file-or-dir> or \
                         `cellgov keys import <file-or-dir>`";

fn decrypt_refusal(path: &str, e: &SceError) -> CommandError {
    if e.is_key_vault_refusal() {
        CommandError::failed(format!("failed to decrypt SELF {path}: {e}\n{KEYS_HINT}"))
    } else {
        CommandError::failed(format!("failed to decrypt SELF {path}: {e}"))
    }
}

/// Plaintext ELF bytes plus the boot identity read from the SELF
/// wrapper before decryption. Both identity fields are `None` for
/// raw-ELF inputs, which have no SELF headers; `control_flags1` is
/// also `None` for a SELF that carries no plaintext capability
/// header, which is the unprivileged case.
pub(crate) struct LoadedPpuImage {
    pub elf_data: Vec<u8>,
    pub authority_id: Option<u64>,
    pub control_flags1: Option<u32>,
}

/// Reads a PPU image and resolves its RAP from the title manifest.
///
/// The operator named the image, so a SELF header that will not parse
/// refuses it.
///
/// # Errors
///
/// Returns an error if the command cannot load the image.
pub(crate) fn load_ppu_image_with_title(
    path: &str,
    title: &TitleManifest,
    vfs_root: &Path,
) -> Result<LoadedPpuImage, CommandError> {
    let bytes = load_file(path)?;
    let keys = super::keys::key_vault_for(&bytes)?;
    let lookup = title.rap_lookup(vfs_root);
    let image = open_ppu_image(bytes, keys, KeyPolicy::Auto(&lookup))
        .map_err(|e| decrypt_refusal(path, &e))?;
    let (authority_id, control_flags1) = match image.identity {
        None => (None, None),
        Some(SelfIdentity {
            authority_id,
            control_flags1,
        }) => (
            Some(authority_id.map_err(|error| {
                CommandError::failed(format!("SELF {path}: identification header: {error}"))
            })?),
            control_flags1.map_err(|error| {
                CommandError::failed(format!("SELF {path}: capability header: {error}"))
            })?,
        ),
    };
    Ok(LoadedPpuImage {
        elf_data: image.elf,
        authority_id,
        control_flags1,
    })
}

/// Distinguishes an absent dump from a broken input.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoadPpuImageError {
    #[error(transparent)]
    NotInstalled(#[from] TitleNotInstalled),
    #[error(transparent)]
    Failed(#[from] CommandError),
}

/// Returns the first loadable image from `eboot_candidates`; see
/// [`TitleManifest::load_eboot`].
///
/// The walk found the image, so a SELF header that will not parse
/// falls back to the retail authority id or to unprivileged, with a
/// warning.
///
/// # Errors
///
/// - Returns `NotInstalled` if no candidate exists.
/// - Returns `Failed` if a present candidate cannot load.
pub(crate) fn load_ppu_image_walk_candidates(
    title: &TitleManifest,
    vfs_root: &Path,
    eboot_dirs: &[PathBuf],
) -> Result<(LoadedPpuImage, PathBuf), LoadPpuImageError> {
    let (image, path) = match title.load_eboot(eboot_dirs, vfs_root, &super::keys::ProcessKeyVault)
    {
        Ok(loaded) => loaded,
        Err(EbootLoadError::NotInstalled(e)) => return Err(e.into()),
        Err(e @ EbootLoadError::Stopped { .. }) if stopped_by_the_vault(&e) => {
            return Err(CommandError::failed(format!("load ppu image: {e}\n{KEYS_HINT}")).into())
        }
        Err(e) => return Err(CommandError::failed(format!("load ppu image: {e}")).into()),
    };
    let (authority_id, control_flags1) = match image.identity {
        None => (None, None),
        Some(SelfIdentity {
            authority_id,
            control_flags1,
        }) => (
            authority_id
                .inspect_err(|e| {
                    eprintln!(
                        "load ppu image: {}: SELF identification header: {e}; \
                         program_authority_id falls back to the retail constant",
                        path.display(),
                    );
                })
                .ok(),
            control_flags1
                .inspect_err(|e| {
                    eprintln!(
                        "load ppu image: {}: SELF capability header: {e}; \
                         ctrl_flags1 falls back to unprivileged",
                        path.display(),
                    );
                })
                .ok()
                .flatten(),
        ),
    };
    Ok((
        LoadedPpuImage {
            elf_data: image.elf,
            authority_id,
            control_flags1,
        },
        path,
    ))
}

fn stopped_by_the_vault(e: &EbootLoadError) -> bool {
    matches!(e, EbootLoadError::Stopped { source, .. } if source.is_key_vault_refusal())
}

#[cfg(test)]
#[path = "tests/exit_tests.rs"]
mod tests;
