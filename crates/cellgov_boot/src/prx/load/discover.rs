//! Firmware module discovery: the directory scan, the manifest and its verification, and the candidate universe.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::prx::types::PrxLoadStageError;
use crate::KeyVaultSource;

use cellgov_ps3_abi::format::dev_flash::FIRMWARE_INTERNAL_PRX_STEMS;

use super::error::FirmwareLoadError;

/// Locate the firmware module file for `stem` under `dir_path`.
///
/// Prefers `.sprx` (SCE-wrapped) over `.prx` (pre-decrypted) so both
/// boot modes converge on the same on-disk file when both exist.
fn find_firmware_module(dir_path: &Path, stem: &str) -> Option<PathBuf> {
    let sprx = dir_path.join(format!("{stem}.sprx"));
    if sprx.is_file() {
        return Some(sprx);
    }
    let prx = dir_path.join(format!("{stem}.prx"));
    if prx.is_file() {
        return Some(prx);
    }
    None
}

/// Every `*.sprx` directly under `dir_path`, sorted.
///
/// # Errors
///
/// The directory or one of its entries could not be read.
fn scan_sprx_files(dir_path: &Path) -> Result<Vec<PathBuf>, FirmwareLoadError> {
    let entries = std::fs::read_dir(dir_path).map_err(|source| FirmwareLoadError::ReadDir {
        dir: dir_path.to_path_buf(),
        source,
    })?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| FirmwareLoadError::ReadDirEntry {
            dir: dir_path.to_path_buf(),
            source,
        })?;
        let p = entry.path();
        if p.extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("sprx"))
        {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Locate and parse `firmware.toml` at or above `dir_path`.
///
/// The install writes the manifest at the install root while boots
/// point at `<root>/sys/external`, so the walk covers the directory
/// itself and two levels up. Returns the root the manifest governs.
///
/// # Errors
///
/// No manifest covers `dir_path`, or the one found does not read or
/// parse.
pub(super) fn locate_and_parse_manifest(
    dir_path: &Path,
) -> Result<(PathBuf, cellgov_install::manifest::FirmwareManifest), FirmwareLoadError> {
    let mut root = dir_path.to_path_buf();
    for _ in 0..3 {
        let candidate = root.join(cellgov_install::manifest::MANIFEST_FILE);
        if candidate.is_file() {
            let text = std::fs::read_to_string(&candidate).map_err(|source| {
                FirmwareLoadError::ManifestRead {
                    path: candidate.clone(),
                    source,
                }
            })?;
            let manifest = cellgov_install::manifest::parse_manifest(&text).map_err(|source| {
                FirmwareLoadError::ManifestParse {
                    path: candidate.clone(),
                    source: Box::new(source),
                }
            })?;
            return Ok((root, manifest));
        }
        if !root.pop() {
            break;
        }
    }
    Err(FirmwareLoadError::NoManifest {
        dir: dir_path.to_path_buf(),
    })
}

/// Install-root-relative, forward-slash form of `file`, matching the
/// manifest's `[[files]].path` convention.
///
/// # Errors
///
/// `file` is not under `root`.
pub(super) fn manifest_rel_path(root: &Path, file: &Path) -> Result<String, FirmwareLoadError> {
    let rel = file
        .strip_prefix(root)
        .map_err(|_| FirmwareLoadError::ModuleOutsideRoot {
            file: file.to_path_buf(),
            root: root.to_path_buf(),
        })?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// Verify one loaded module's post-decrypt bytes against the manifest.
///
/// # Errors
///
/// The module is unlisted, or its digest does not match the one the
/// manifest records.
pub(super) fn verify_against_manifest(
    manifest: &cellgov_install::manifest::FirmwareManifest,
    root: &Path,
    file: &Path,
    elf: &[u8],
) -> Result<(), FirmwareLoadError> {
    use cellgov_install::manifest::{sha256_of, verify_post_decrypt, Sha256, VerifyOutcome};
    let rel = manifest_rel_path(root, file)?;
    match verify_post_decrypt(manifest, &rel, &sha256_of(elf)) {
        VerifyOutcome::Match => Ok(()),
        VerifyOutcome::NotInManifest => Err(FirmwareLoadError::ModuleNotInManifest {
            file: file.to_path_buf(),
            rel,
        }),
        VerifyOutcome::Mismatch { expected, actual } => {
            Err(FirmwareLoadError::ModuleDigestMismatch {
                file: file.to_path_buf(),
                expected: Sha256(expected).to_hex(),
                actual: Sha256(actual).to_hex(),
            })
        }
    }
}

/// Read a firmware module file and decrypt if SCE-wrapped. Returns
/// the raw bytes otherwise so pre-decrypted `.prx` files load through
/// the same path.
fn read_firmware_module_elf(
    path: &Path,
    keys: &dyn KeyVaultSource,
) -> Result<Vec<u8>, PrxLoadStageError> {
    let raw = std::fs::read(path).map_err(|source| PrxLoadStageError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    // A firmware tree carries no NPDRM content and no RAP to resolve
    // against, so its SELFs are APP-keyed by construction.
    let keys = keys
        .vault_for(&raw)
        .map_err(|e| PrxLoadStageError::KeyVault {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;
    cellgov_install::self_image::into_plaintext_elf(
        raw,
        keys,
        cellgov_install::self_image::KeyPolicy::AppOnly,
    )
    .map_err(|source| PrxLoadStageError::Decrypt {
        path: path.to_path_buf(),
        source,
    })
}

/// The firmware module universe one selection runs over: every
/// `.sprx` under the firmware directory (plus, for a firmware
/// executable, `sys/internal/`), decrypted once, with the manifest
/// that vouches for it.
pub struct FirmwareCandidates {
    pub(super) root: PathBuf,
    pub(super) manifest: cellgov_install::manifest::FirmwareManifest,
    /// Path -> post-decrypt ELF bytes.
    pub(super) modules: BTreeMap<String, Vec<u8>>,
    /// `(stem, path)` of the sys/internal modules a firmware executable
    /// cannot boot without; empty unless `include_internal`.
    pub(super) internal_paths: Vec<(&'static str, String)>,
    /// Whether the load set is every viable candidate (a firmware
    /// executable builds its import tables at runtime and names no
    /// roots) or the closure of the image's own import table.
    pub(super) include_internal: bool,
}

impl FirmwareCandidates {
    /// Scan and decrypt `dir`.
    ///
    /// # Errors
    ///
    /// - A directory the scan walks does not read.
    /// - A module does not read or decrypt.
    /// - No `firmware.toml` covers the tree.
    /// - A stem a firmware executable needs is absent.
    pub fn scan(
        dir: &str,
        include_internal: bool,
        keys: &dyn KeyVaultSource,
    ) -> Result<Self, FirmwareLoadError> {
        let dir_path = std::path::PathBuf::from(dir);
        let (fw_root, fw_manifest) = locate_and_parse_manifest(&dir_path)?;

        // Candidate universe: every module in the firmware directory,
        // plus -- for a firmware executable -- the sys/internal stems the
        // shell loads by path at runtime. Their imports participate in
        // viability like anyone else's.
        let mut candidates: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for path in scan_sprx_files(&dir_path)? {
            let elf = read_firmware_module_elf(&path, keys)?;
            candidates.insert(utf8_path(&path)?, elf);
        }
        if candidates.is_empty() {
            return Err(FirmwareLoadError::NoModules { dir: dir_path });
        }
        let mut internal_paths: Vec<(&str, String)> = Vec::new();
        if include_internal {
            let internal_dir = fw_root.join("sys").join("internal");
            // The shell loads internal modules by guest path at runtime
            // (sc 480), so the whole directory is candidate material;
            // viability prunes what does not close, with the reason
            // reported. A prune is fatal only for the
            // FIRMWARE_INTERNAL_PRX_STEMS entries, which the shell cannot
            // boot without.
            for path in scan_sprx_files(&internal_dir)? {
                let elf = read_firmware_module_elf(&path, keys)?;
                candidates.insert(utf8_path(&path)?, elf);
            }
            for stem in FIRMWARE_INTERNAL_PRX_STEMS {
                let path = find_firmware_module(&internal_dir, stem).ok_or_else(|| {
                    FirmwareLoadError::InternalStemAbsent {
                        stem,
                        dir: internal_dir.clone(),
                    }
                })?;
                let path_str = utf8_path(&path)?;
                // The directory scan covers .sprx; a pre-decrypted .prx
                // resolved by the stem walk still needs its bytes read.
                if !candidates.contains_key(&path_str) {
                    let elf = read_firmware_module_elf(&path, keys)?;
                    candidates.insert(path_str.clone(), elf);
                }
                internal_paths.push((stem, path_str));
            }
        }
        Ok(Self {
            root: fw_root,
            manifest: fw_manifest,
            modules: candidates,
            internal_paths,
            include_internal,
        })
    }
}

/// The path as the candidate map keys it.
///
/// # Errors
///
/// The path is not Unicode.
fn utf8_path(path: &Path) -> Result<String, FirmwareLoadError> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| FirmwareLoadError::NonUtf8Path {
            path: path.to_path_buf(),
        })
}
