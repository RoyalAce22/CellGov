//! SELF loading, decryption, and title-image selection.

use std::path::{Path, PathBuf};
use std::{cell::RefCell, rc::Rc};

use cellgov_install::npdrm::{NpdHeaderInfo, Rap};
use cellgov_install::sce::SceError;
use cellgov_install::self_image::{is_sce_wrapped, to_plaintext_elf, KeyPolicy};
use cellgov_ps3_abi::format::elf::ELF_MAGIC;

use cellgov_boot::manifest::{ResolveEbootError, TitleManifest};

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

pub(crate) fn decrypt_ppu_self(
    bytes: &[u8],
    path: &str,
    vfs_root: &Path,
) -> Result<Vec<u8>, CommandError> {
    let exdata = super::title::exdata_dir(vfs_root);
    let resolver_error = Rc::new(RefCell::new(None));
    let resolver_error_for_closure = Rc::clone(&resolver_error);
    let resolver = |npd: &NpdHeaderInfo| -> Option<Rap> {
        let rap_path = exdata.join(format!("{}.rap", npd.content_id));
        // Only "no such file" is an absent RAP. Any other read failure
        // (permissions, a directory in its place) would otherwise take
        // the same silent path and boot the title on the vault's free
        // klicensee.
        let rap_bytes = match std::fs::read(&rap_path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                *resolver_error_for_closure.borrow_mut() = Some(CommandError::failed(format!(
                    "failed to read RAP for NPDRM content {} at {}: {}",
                    npd.content_id,
                    rap_path.display(),
                    error,
                )));
                return None;
            }
        };
        let rap_arr: [u8; 16] = match rap_bytes.as_slice().try_into() {
            Ok(value) => value,
            Err(_) => {
                *resolver_error_for_closure.borrow_mut() = Some(CommandError::failed(format!(
                    "RAP file {} is {} bytes; expected exactly 16",
                    rap_path.display(),
                    rap_bytes.len(),
                )));
                return None;
            }
        };
        Some(Rap(rap_arr))
    };
    let plaintext = to_plaintext_elf(
        bytes,
        super::keys::key_vault_for(bytes)?,
        KeyPolicy::Auto(&resolver),
    );
    if let Some(error) = resolver_error.borrow_mut().take() {
        return Err(error);
    }
    match plaintext {
        Ok(elf) => Ok(elf.into_owned()),
        Err(e @ SceError::NoRapForNpdrmTitle { .. }) => Err(CommandError::failed(format!(
            "{e}; expected its RAP at {}/<content_id>.rap",
            exdata.display()
        ))),
        Err(e) if is_key_vault_refusal(&e) => Err(CommandError::failed(format!(
            "failed to decrypt SELF {path}: {e}\n{KEYS_HINT}"
        ))),
        Err(e) => Err(CommandError::failed(format!(
            "failed to decrypt SELF {path}: {e}"
        ))),
    }
}

/// The one line every vault refusal ends with.
const KEYS_HINT: &str = "supply keys with CELLGOV_KEYS=<file-or-dir> or \
                         `cellgov keys import <file-or-dir>`";

/// A refusal every SCE-wrapped image in the run answers the same: the
/// vault did not load, or holds no keyset for the image's class and
/// revision.
fn is_key_vault_refusal(e: &SceError) -> bool {
    matches!(
        e,
        SceError::Keys(_)
            | SceError::NoAppKey { .. }
            | SceError::NoNpdrmKey { .. }
            | SceError::NoLv2Key { .. }
            | SceError::RapPboxNotAPermutation { .. }
    )
}

#[derive(Debug, thiserror::Error)]
enum LoadCandidateError {
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Decrypt(#[from] SceError),
    #[error("bytes are not a SELF or plaintext ELF")]
    NotElf,
}

fn rap_resolver(
    title: &TitleManifest,
    vfs_root: PathBuf,
    error: Rc<RefCell<Option<CommandError>>>,
) -> impl Fn(&NpdHeaderInfo) -> Option<Rap> {
    let rap_filename = title.rap_filename.clone();
    move |npd: &NpdHeaderInfo| -> Option<Rap> {
        let rap_filename = rap_filename.as_ref()?;
        let rap_path = super::title::exdata_dir(&vfs_root).join(rap_filename);
        let rap_bytes = match std::fs::read(&rap_path) {
            Ok(bytes) => bytes,
            Err(read_error) => {
                *error.borrow_mut() = Some(CommandError::failed(format!(
                    "failed to read RAP for NPDRM title {} (license {}) at {}: {}",
                    npd.content_id,
                    npd.license as u32,
                    rap_path.display(),
                    read_error,
                )));
                return None;
            }
        };
        let rap_arr: [u8; 16] = match rap_bytes.as_slice().try_into() {
            Ok(value) => value,
            Err(_) => {
                *error.borrow_mut() = Some(CommandError::failed(format!(
                    "RAP file {} is {} bytes; expected exactly 16",
                    rap_path.display(),
                    rap_bytes.len(),
                )));
                return None;
            }
        };
        Some(Rap(rap_arr))
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
/// # Errors
///
/// Returns an error if the command cannot load the image.
pub(crate) fn load_ppu_image_with_title(
    path: &str,
    title: &TitleManifest,
    vfs_root: &Path,
) -> Result<LoadedPpuImage, CommandError> {
    let bytes = load_file(path)?;
    if !is_sce_wrapped(&bytes) {
        return Ok(LoadedPpuImage {
            elf_data: bytes,
            authority_id: None,
            control_flags1: None,
        });
    }
    let authority_id = Some(
        cellgov_install::sce::parse_program_authority_id(&bytes).map_err(|error| {
            CommandError::failed(format!("SELF {path}: identification header: {error}"))
        })?,
    );
    let control_flags1 = cellgov_install::sce::parse_control_flags1(&bytes).map_err(|error| {
        CommandError::failed(format!("SELF {path}: capability header: {error}"))
    })?;
    let resolver_error = Rc::new(RefCell::new(None));
    let resolver = rap_resolver(title, vfs_root.to_path_buf(), Rc::clone(&resolver_error));
    let plaintext = to_plaintext_elf(
        &bytes,
        super::keys::key_vault_for(&bytes)?,
        KeyPolicy::Auto(&resolver),
    );
    if let Some(error) = resolver_error.borrow_mut().take() {
        return Err(error);
    }
    let elf_data = plaintext
        .map_err(|error| {
            if is_key_vault_refusal(&error) {
                CommandError::failed(format!(
                    "failed to decrypt SELF {path}: {error}\n{KEYS_HINT}"
                ))
            } else {
                CommandError::failed(format!("failed to decrypt SELF {path}: {error}"))
            }
        })?
        .into_owned();
    Ok(LoadedPpuImage {
        elf_data,
        authority_id,
        control_flags1,
    })
}

/// Represents an absent title dump.
///
/// A present file that fails to load is a [`LoadPpuImageError::Failed`].
#[derive(Debug, thiserror::Error)]
pub(crate) enum TitleNotInstalled {
    /// Every candidate is a plain miss in every content directory
    /// probed; see [`is_plain_miss`].
    #[error("resolve_eboot for title {title}: {source}")]
    NoContentDirectory {
        title: String,
        /// Boxed: its not-found variant carries four probe lists.
        #[source]
        source: Box<ResolveEbootError>,
    },
    /// The content directory exists and every candidate is missing.
    #[error("every eboot_candidate for title {title} failed under {usrdir}:\n{attempts}")]
    NoEbootCandidate {
        title: String,
        usrdir: String,
        /// One line per candidate, each with its own read failure.
        attempts: String,
    },
}

impl TitleNotInstalled {
    pub(crate) fn marker_note(&self) -> &'static str {
        match self {
            Self::NoContentDirectory { .. } => "no content directory",
            Self::NoEbootCandidate { .. } => "no eboot candidate present",
        }
    }
}

/// Distinguishes an absent dump from a broken input.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoadPpuImageError {
    #[error(transparent)]
    NotInstalled(#[from] TitleNotInstalled),
    #[error(transparent)]
    Failed(#[from] CommandError),
}

/// Whether a probe refusal says only that nothing is there.
///
/// The probe folds three other findings into the same variant, and
/// none of them is an absence:
///
/// - a name taken by a directory or special file;
/// - a metadata read that failed for a reason other than not-found;
/// - a manifest that lists nothing to probe.
///
/// Each is a dump or manifest that is present and wrong, so it takes
/// the broken-dump path.
fn is_plain_miss(e: &ResolveEbootError) -> bool {
    matches!(
        e,
        ResolveEbootError::NotFound {
            searched: _,
            candidates,
            probe_errors,
            not_regular,
        } if !candidates.is_empty() && probe_errors.is_empty() && not_regular.is_empty()
    )
}

/// Returns the first plaintext ELF from `eboot_candidates`.
///
/// A decrypt refusal stops the walk so a plaintext candidate cannot
/// replace the canonical SCE image.
///
/// The walk runs in the first entry of `eboot_dirs` that holds any
/// candidate. A selected update's executable therefore shadows the
/// base one, and the two directories never interleave.
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
    let resolved = match title.resolve_eboot_in(eboot_dirs) {
        Ok(p) => p,
        // A plain miss in every probed directory: the dump is not on
        // this machine. Anything else is present and wrong; see
        // `is_plain_miss`.
        Err(e) if is_plain_miss(&e) => {
            return Err(TitleNotInstalled::NoContentDirectory {
                title: title.name().to_string(),
                source: Box::new(e),
            }
            .into())
        }
        Err(e) => {
            return Err(CommandError::failed(format!(
                "load ppu image: resolve_eboot for title {}: {e}",
                title.name(),
            ))
            .into())
        }
    };
    let usrdir = resolved.parent().ok_or_else(|| {
        CommandError::failed("load ppu image: resolved EBOOT has no parent directory")
    })?;
    let usrdir = usrdir.to_path_buf();

    let resolver_error = Rc::new(RefCell::new(None));
    let resolver = rap_resolver(title, vfs_root.to_path_buf(), Rc::clone(&resolver_error));
    let mut attempts: Vec<(String, LoadCandidateError)> = Vec::new();
    for candidate in &title.eboot_candidates {
        let path = usrdir.join(candidate);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                attempts.push((candidate.clone(), LoadCandidateError::Io(e)));
                continue;
            }
        };
        if is_sce_wrapped(&bytes) {
            // Both headers are plaintext, so a parse failure here is a
            // structural anomaly in a file the walk is about to boot;
            // each refusal is named rather than folded into `None`.
            let authority_id = match cellgov_install::sce::parse_program_authority_id(&bytes) {
                Ok(id) => Some(id),
                Err(e) => {
                    eprintln!(
                        "load ppu image: {}: SELF identification header: {e}; \
                         program_authority_id falls back to the retail constant",
                        path.display(),
                    );
                    None
                }
            };
            let control_flags1 = match cellgov_install::sce::parse_control_flags1(&bytes) {
                Ok(flags) => flags,
                Err(e) => {
                    eprintln!(
                        "load ppu image: {}: SELF capability header: {e}; \
                         ctrl_flags1 falls back to unprivileged",
                        path.display(),
                    );
                    None
                }
            };
            let plaintext = to_plaintext_elf(
                &bytes,
                super::keys::key_vault_for(&bytes).map_err(LoadPpuImageError::Failed)?,
                KeyPolicy::Auto(&resolver),
            );
            if let Some(error) = resolver_error.borrow_mut().take() {
                return Err(error.into());
            }
            match plaintext {
                Ok(elf) => {
                    return Ok((
                        LoadedPpuImage {
                            elf_data: elf.into_owned(),
                            authority_id,
                            control_flags1,
                        },
                        path,
                    ))
                }
                // The next candidate answers the same if it is
                // SCE-wrapped, and a plaintext one would boot in place
                // of the canonical binary without a word.
                Err(e @ SceError::DecryptFeatureDisabled) => {
                    return Err(CommandError::failed(format!(
                        "load ppu image: {}: {e}; no later eboot_candidate is tried \
                         for title {}",
                        path.display(),
                        title.name(),
                    ))
                    .into())
                }
                // Same reasoning; see `is_key_vault_refusal`.
                Err(e) if is_key_vault_refusal(&e) => {
                    return Err(CommandError::failed(format!(
                        "load ppu image: {}: {e}; no later eboot_candidate is tried \
                         for title {}\n{KEYS_HINT}",
                        path.display(),
                        title.name(),
                    ))
                    .into())
                }
                Err(e) => {
                    attempts.push((candidate.clone(), LoadCandidateError::Decrypt(e)));
                    continue;
                }
            }
        } else if bytes.len() >= 4 && bytes[..4] == ELF_MAGIC {
            return Ok((
                LoadedPpuImage {
                    elf_data: bytes,
                    authority_id: None,
                    control_flags1: None,
                },
                path,
            ));
        } else {
            attempts.push((candidate.clone(), LoadCandidateError::NotElf));
            continue;
        }
    }
    // Missing dump vs broken dump: only when every candidate failed
    // because the file does not exist is the title "not installed".
    let all_missing = attempts.iter().all(|(_, why)| {
        matches!(why, LoadCandidateError::Io(e) if e.kind() == std::io::ErrorKind::NotFound)
    });
    let usrdir_str = usrdir.display().to_string();
    let attempts_str = attempts
        .iter()
        .map(|(name, why)| format!("    {name}: {why}"))
        .collect::<Vec<_>>()
        .join("\n");
    if all_missing {
        return Err(TitleNotInstalled::NoEbootCandidate {
            title: title.name().to_string(),
            usrdir: usrdir_str,
            attempts: attempts_str,
        }
        .into());
    }
    Err(CommandError::failed(format!(
        "load ppu image: every eboot_candidate for title {} failed under {usrdir_str}:\n{attempts_str}",
        title.name(),
    ))
    .into())
}

#[cfg(test)]
#[path = "tests/exit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/exit_not_installed_tests.rs"]
mod not_installed_tests;
