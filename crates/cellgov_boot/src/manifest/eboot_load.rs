//! The walk that opens a title's executable from its
//! `eboot_candidates`, and the rule that tells a dump that is not on
//! this machine from one that is present and broken.

use std::path::{Path, PathBuf};

use cellgov_install::npdrm::{read_rap, NpdHeaderInfo, Rap, RapPresence, RapReadError};
use cellgov_install::sce::SceError;
use cellgov_install::self_image::{is_sce_wrapped, open_ppu_image, KeyPolicy, PpuImage};
use cellgov_install::store::hdd0_exdata_dir;
use cellgov_ps3_abi::format::elf::ELF_MAGIC;

use super::{ResolveEbootError, TitleManifest};
use crate::KeyVaultSource;

/// A title whose dump is not on this machine.
///
/// A present file that fails to load is an [`EbootLoadError`] of
/// another variant.
#[derive(Debug, thiserror::Error)]
pub enum TitleNotInstalled {
    /// Every candidate is a plain miss in every content directory
    /// probed; see [`ResolveEbootError::is_plain_miss`].
    #[error("resolve_eboot for title {title}: {source}")]
    NoContentDirectory {
        /// The title's name.
        title: String,
        /// Boxed: its not-found variant carries four probe lists.
        #[source]
        source: Box<ResolveEbootError>,
    },
    /// The content directory exists and every candidate is missing.
    #[error("every eboot_candidate for title {title} failed under {usrdir}:\n{attempts}")]
    NoEbootCandidate {
        /// The title's name.
        title: String,
        /// The content directory the walk ran in.
        usrdir: String,
        /// One line per candidate, each with its own read failure.
        attempts: String,
    },
}

impl TitleNotInstalled {
    /// The short reason a suite's skip marker records.
    pub fn marker_note(&self) -> &'static str {
        match self {
            Self::NoContentDirectory { .. } => "no content directory",
            Self::NoEbootCandidate { .. } => "no eboot candidate present",
        }
    }
}

/// Why [`TitleManifest::load_eboot`] opened no image.
#[derive(Debug, thiserror::Error)]
pub enum EbootLoadError {
    /// The dump is not on this machine.
    #[error(transparent)]
    NotInstalled(#[from] TitleNotInstalled),
    /// The probe found a dump or manifest that is present and wrong.
    #[error("resolve_eboot for title {title}: {source}")]
    Resolve {
        /// The title's name.
        title: String,
        /// The probe's refusal.
        #[source]
        source: Box<ResolveEbootError>,
    },
    /// The resolved executable has no parent directory to walk.
    #[error("resolved EBOOT {} has no parent directory", path.display())]
    NoParent {
        /// The resolved executable.
        path: PathBuf,
    },
    /// The operator's vault did not load for an SCE-wrapped candidate.
    #[error("{}: key vault: {reason}", path.display())]
    Vault {
        /// The candidate the walk asked the vault for.
        path: PathBuf,
        /// The vault's refusal, rendered.
        reason: String,
    },
    /// A refusal every later candidate would answer the same, so the
    /// walk stops rather than boot a plaintext candidate in place of
    /// the canonical image.
    #[error("{}: {source}; no later eboot_candidate is tried for title {title}", path.display())]
    Stopped {
        /// The candidate the decrypt refused.
        path: PathBuf,
        /// The title's name.
        title: String,
        /// The refusal; boxed, since a RAP refusal carries a path and
        /// an I/O error.
        #[source]
        source: Box<SceError>,
    },
    /// Every candidate failed, and at least one is present.
    #[error("every eboot_candidate for title {title} failed under {usrdir}:\n{attempts}")]
    AllFailed {
        /// The title's name.
        title: String,
        /// The content directory the walk ran in.
        usrdir: String,
        /// One line per candidate, each with its own failure.
        attempts: String,
    },
}

/// Why one candidate did not load.
#[derive(Debug, thiserror::Error)]
enum CandidateFailure {
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Decrypt(#[from] SceError),
    #[error("bytes are not a SELF or plaintext ELF")]
    NotElf,
}

impl ResolveEbootError {
    /// Whether this refusal says only that nothing is there.
    ///
    /// The probe folds three other findings into the same variant, and
    /// none of them is an absence:
    ///
    /// - a name taken by a directory or special file;
    /// - a metadata read that failed for a reason other than not-found;
    /// - a manifest that lists nothing to probe.
    ///
    /// Each is a dump or manifest that is present and wrong.
    pub fn is_plain_miss(&self) -> bool {
        matches!(
            self,
            ResolveEbootError::NotFound {
                searched: _,
                candidates,
                probe_errors,
                not_regular,
            } if !candidates.is_empty() && probe_errors.is_empty() && not_regular.is_empty()
        )
    }
}

/// Whether a candidate's decrypt refusal stops the walk.
///
/// The feature and the vault answer every candidate the same, and a
/// refused RAP is the title's, not the candidate's.
fn stops_the_walk(e: &SceError) -> bool {
    matches!(
        e,
        SceError::DecryptFeatureDisabled | SceError::RapRead { .. }
    ) || e.is_key_vault_refusal()
}

impl TitleManifest {
    /// The RAP lookup for this title's NPDRM image: the file the
    /// manifest names under the exdata directory of `vfs_root`.
    ///
    /// A title that names no RAP answers `Ok(None)`. The lookup refuses
    /// a named file that is missing rather than reading it as absent.
    pub fn rap_lookup(
        &self,
        vfs_root: &Path,
    ) -> impl Fn(&NpdHeaderInfo) -> Result<Option<Rap>, RapReadError> + use<'_> {
        let exdata = hdd0_exdata_dir(vfs_root);
        move |_: &NpdHeaderInfo| match &self.rap_filename {
            Some(name) => read_rap(&exdata.join(name), RapPresence::Required),
            None => Ok(None),
        }
    }

    /// Open the first loadable image among `eboot_candidates`.
    ///
    /// The walk runs in the first entry of `dirs` that holds any
    /// candidate, so a selected update's executable shadows the base
    /// one, and the two directories never interleave. A refusal that
    /// every candidate would answer the same stops the walk, so a
    /// plaintext candidate cannot replace the canonical SCE image.
    ///
    /// # Errors
    ///
    /// [`EbootLoadError::NotInstalled`] when no candidate exists;
    /// another variant when a present candidate cannot load.
    pub fn load_eboot(
        &self,
        dirs: &[PathBuf],
        vfs_root: &Path,
        keys: &dyn KeyVaultSource,
    ) -> Result<(PpuImage, PathBuf), EbootLoadError> {
        let resolved = match self.resolve_eboot_in(dirs) {
            Ok(p) => p,
            Err(e) if e.is_plain_miss() => {
                return Err(TitleNotInstalled::NoContentDirectory {
                    title: self.name().to_string(),
                    source: Box::new(e),
                }
                .into())
            }
            Err(e) => {
                return Err(EbootLoadError::Resolve {
                    title: self.name().to_string(),
                    source: Box::new(e),
                })
            }
        };
        let usrdir = resolved
            .parent()
            .ok_or_else(|| EbootLoadError::NoParent {
                path: resolved.clone(),
            })?
            .to_path_buf();

        let lookup = self.rap_lookup(vfs_root);
        let mut attempts: Vec<(&str, CandidateFailure)> = Vec::new();
        for candidate in &self.eboot_candidates {
            let path = usrdir.join(candidate);
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    attempts.push((candidate, CandidateFailure::Io(e)));
                    continue;
                }
            };
            if !is_sce_wrapped(&bytes) && !bytes.starts_with(&ELF_MAGIC) {
                attempts.push((candidate, CandidateFailure::NotElf));
                continue;
            }
            let vault = keys.vault_for(&bytes).map_err(|e| EbootLoadError::Vault {
                path: path.clone(),
                reason: e.to_string(),
            })?;
            match open_ppu_image(bytes, vault, KeyPolicy::Auto(&lookup)) {
                Ok(image) => return Ok((image, path)),
                Err(e) if stops_the_walk(&e) => {
                    return Err(EbootLoadError::Stopped {
                        path,
                        title: self.name().to_string(),
                        source: Box::new(e),
                    })
                }
                Err(e) => attempts.push((candidate, CandidateFailure::Decrypt(e))),
            }
        }
        // Missing dump vs broken dump: only when every candidate failed
        // because the file does not exist is the title not installed.
        let all_missing = attempts.iter().all(|(_, why)| {
            matches!(why, CandidateFailure::Io(e) if e.kind() == std::io::ErrorKind::NotFound)
        });
        let title = self.name().to_string();
        let usrdir = usrdir.display().to_string();
        let attempts = attempts
            .iter()
            .map(|(name, why)| format!("    {name}: {why}"))
            .collect::<Vec<_>>()
            .join("\n");
        if all_missing {
            return Err(TitleNotInstalled::NoEbootCandidate {
                title,
                usrdir,
                attempts,
            }
            .into());
        }
        Err(EbootLoadError::AllFailed {
            title,
            usrdir,
            attempts,
        })
    }
}

#[cfg(test)]
#[path = "tests/eboot_load_tests.rs"]
mod tests;
