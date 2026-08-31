//! Building `firmware.toml` over a staged firmware tree.

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the manifest builder is reachable only from the gated installer; the feature-on build lints it"
    )
)]

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::elf::ELF_MAGIC;
use sha2::{Digest, Sha256};

use super::error::FirmwareInstallError;
use crate::keys::KeyVault;
use crate::manifest::{
    self, FirmwareFileEntry, FirmwareIdentity, FirmwareManifest, SUPPORTED_FORMAT_VERSION,
};
use crate::store::layout::VersionKey;
use crate::{sce, self_image};

/// A module the manifest could not cover, and why.
#[derive(Debug, Clone)]
pub enum ManifestOmission {
    /// The SCE container would not decrypt (typically a revision the
    /// vault holds no APP key for).
    Undecryptable {
        /// Tree-relative path of the module.
        path: String,
        /// Why the decrypt failed, already rendered.
        reason: String,
    },
    /// The file is neither an SCE container nor a bare ELF, so it
    /// carries no module image to hash.
    ///
    /// PS3 firmware ships at least one zero-byte `.sprx` placeholder;
    /// recording it would put the empty-bytes hash in the manifest as
    /// though an empty file were a module the boot verifier could load.
    NotAModule {
        /// Tree-relative path of the file.
        path: String,
        /// Its byte length.
        len: usize,
    },
}

/// Every `.sprx` / `.prx` under `dir`, as `(host path, tree-relative
/// `/`-separated path)`.
///
/// Each level is walked in sorted order, so the result is a pure
/// function of the tree.
fn collect_modules(
    dir: &Path,
    rel: &str,
    out: &mut Vec<(PathBuf, String)>,
) -> Result<(), FirmwareInstallError> {
    let read_err = |source| FirmwareInstallError::Io {
        op: "read dir",
        path: dir.to_path_buf(),
        source,
    };
    let mut sorted: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(read_err)?
        .map(|e| e.map(|e| e.path()).map_err(read_err))
        .collect::<Result<_, _>>()?;
    sorted.sort();
    for p in sorted {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| FirmwareInstallError::NonUtf8Path { path: p.clone() })?;
        let child_rel = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        // `Path::is_dir` answers false both for a regular file and for
        // a stat that failed, and the second would drop a whole subtree
        // from the manifest without naming it.
        let meta = p.metadata().map_err(|source| FirmwareInstallError::Io {
            op: "stat",
            path: p.clone(),
            source,
        })?;
        if meta.is_dir() {
            collect_modules(&p, &child_rel, out)?;
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("sprx") || x.eq_ignore_ascii_case("prx"))
        {
            out.push((p, child_rel));
        }
    }
    Ok(())
}

/// Build the `firmware.toml` covering `dev_flash_dir`.
///
/// Per-file hashes are over the post-decrypt ELF bytes. A module the
/// manifest cannot cover is left out and named in the returned
/// omissions -- a tally alone cannot distinguish an expected
/// missing-key skip from a corrupt install.
///
/// # Errors
///
/// [`FirmwareInstallError::Io`] when the walk cannot list or stat part
/// of the tree, [`FirmwareInstallError::ModuleReadFailed`] for a module that
/// will not read, and [`FirmwareInstallError::NonUtf8Path`] for a path
/// `firmware.toml` cannot name.
#[cfg(feature = "decrypt")]
pub(super) fn build_manifest(
    pup_sha256: manifest::Sha256,
    pup_image_version: u64,
    version: &VersionKey,
    dev_flash_dir: &Path,
    keys: &KeyVault,
) -> Result<(FirmwareManifest, Vec<ManifestOmission>), FirmwareInstallError> {
    let mut modules = Vec::new();
    collect_modules(dev_flash_dir, "", &mut modules)?;

    let mut files = Vec::with_capacity(modules.len());
    let mut omissions = Vec::new();
    for (host_path, path) in modules {
        let raw =
            std::fs::read(&host_path).map_err(|source| FirmwareInstallError::ModuleReadFailed {
                path: host_path.clone(),
                source,
            })?;
        let (elf, revision) = if self_image::is_sce_wrapped(&raw) {
            let elf = match sce::decrypt_self_to_elf(&raw, keys) {
                Ok(e) => e,
                Err(source) => {
                    omissions.push(ManifestOmission::Undecryptable {
                        path,
                        reason: source.to_string(),
                    });
                    continue;
                }
            };
            // decrypt_self_to_elf already parsed the same header to get
            // here, so this parse cannot fail.
            let revision = sce::parse_sce_header(&raw)
                .expect("decrypt_self_to_elf success implies parse_sce_header success")
                .revision_flags
                & 0x7FFF;
            (elf, revision)
        } else if raw.starts_with(&ELF_MAGIC) {
            // A pre-decrypted `.prx` carries no SCE wrapper, so its raw
            // bytes are its post-decrypt image and the revision the
            // wrapper held is gone.
            (raw, 0)
        } else {
            omissions.push(ManifestOmission::NotAModule {
                path,
                len: raw.len(),
            });
            continue;
        };
        let mut h = Sha256::new();
        h.update(&elf);
        files.push(FirmwareFileEntry {
            path,
            sha256: manifest::Sha256(h.finalize().into()),
            revision,
        });
    }

    Ok((
        FirmwareManifest {
            format_version: SUPPORTED_FORMAT_VERSION,
            firmware: FirmwareIdentity {
                // PUP-header `image_version` is an opaque u64
                // identifier. The user-facing `version` beside it
                // comes from the extracted tree's
                // `vsh/etc/version.txt`.
                image_version: format!("0x{pup_image_version:016x}"),
                version: version.as_str().to_string(),
                pup_sha256,
            },
            files,
        },
        omissions,
    ))
}

#[cfg(test)]
#[path = "tests/manifest_build_tests.rs"]
mod tests;
