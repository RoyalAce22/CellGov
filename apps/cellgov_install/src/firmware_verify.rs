//! Hold an installed firmware tree against the `firmware.toml` inside
//! it.
//!
//! The manifest records each module's post-decrypt hash, so the check
//! decrypts every entry the manifest names. A build without the
//! `decrypt` feature cannot open the SCE wrappers, so it cannot run
//! this check.

use std::path::{Path, PathBuf};

use crate::keys::KeyVault;
use crate::manifest::{self, FirmwareManifest, ManifestError, Sha256 as HexSha256};

/// `firmware.toml` sits inside the mount it covers.
const MANIFEST_FILE: &str = "firmware.toml";

/// Why one module did not match the manifest entry that names it.
///
/// A manifest entry's hash covers the post-decrypt image. A file that
/// yields no image has no hash to compare against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleDivergence {
    /// The manifest names the module; the tree does not hold it.
    Missing,
    /// The module opened, and its image is not the one recorded.
    Modified {
        /// Hash the manifest holds.
        expected: HexSha256,
        /// Hash of the image the file yields now.
        found: HexSha256,
    },
    /// The file is there and yields no module image at all.
    NoImage {
        /// Why the file yields no image.
        reason: String,
    },
}

/// One module that did not match its manifest entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleFault {
    /// The module path the manifest entry names.
    pub path: PathBuf,
    /// How it diverged.
    pub kind: ModuleDivergence,
}

impl std::fmt::Display for ModuleFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let at = self.path.display();
        match &self.kind {
            ModuleDivergence::Missing => write!(f, "{at}: missing"),
            ModuleDivergence::Modified { expected, found } => write!(
                f,
                "{at}: modified (recorded {}, found {})",
                expected.to_hex(),
                found.to_hex()
            ),
            ModuleDivergence::NoImage { reason } => write!(f, "{at}: no module image ({reason})"),
        }
    }
}

/// What a firmware verification pass examined.
///
/// `matched + divergences.len()` is the number of manifest entries
/// checked, so a report cannot hide an entry it neither matched nor
/// named.
#[derive(Debug, Clone, Default)]
pub struct FirmwareVerifyReport {
    /// Manifest entries whose image matched.
    pub matched: usize,
    /// Manifest entries that did not match, in manifest order.
    pub divergences: Vec<ModuleFault>,
}

impl FirmwareVerifyReport {
    /// Manifest entries this pass examined.
    #[must_use]
    pub fn checked(&self) -> usize {
        self.matched + self.divergences.len()
    }

    /// Whether every manifest entry matched.
    ///
    /// The report covers the manifest entries only. See
    /// [`verify_firmware_tree`] for what the manifest leaves out.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.divergences.is_empty()
    }
}

/// Why an installed firmware tree could not be verified.
#[derive(Debug, thiserror::Error)]
pub enum FirmwareVerifyError {
    /// The mount holds no `firmware.toml`, so nothing names what the
    /// tree should hold.
    #[error("firmware mount {} holds no {MANIFEST_FILE}", dir.display())]
    NoManifest {
        /// The mount that was walked.
        dir: PathBuf,
    },
    /// The manifest is there and could not be read.
    #[error("read {}: {source}", path.display())]
    ManifestRead {
        /// The manifest file.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The manifest does not parse, or declares a schema this build
    /// does not read.
    #[error("{}: {source}", path.display())]
    ManifestParse {
        /// The manifest file.
        path: PathBuf,
        /// Why it was refused.
        #[source]
        source: Box<ManifestError>,
    },
    /// A module the manifest names could be neither hashed nor shown
    /// absent.
    #[error("verify-read {}: {source}", path.display())]
    ModuleRead {
        /// The module that could not be read.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The vault cannot open a module the manifest covers, so the pass
    /// cannot say whether that module still matches.
    ///
    /// The gap is one of:
    ///
    /// - no keyset for the revision
    /// - no candidate keyset that opens the envelope
    /// - no klicensee
    #[error(
        "{}: {source}; the vault that installed this firmware held the key, so verify under \
         that vault rather than reading the module as modified",
        path.display()
    )]
    ModuleKeyMissing {
        /// The module that could not be opened.
        path: PathBuf,
        /// The refusal the decrypt raised.
        #[source]
        source: Box<crate::sce::SceError>,
    },
    /// A `[[files]]` entry names a path that would not stay inside the
    /// mount it is joined onto.
    ///
    /// The entry path aims the read the pass hashes, and the manifest
    /// lives inside the tree it describes.
    #[error(
        "{}: entry {entry:?} is not a path inside the mount; reinstall the version with \
         `cellgov firmware install <PS3UPDAT.PUP>`",
        path.display()
    )]
    UnsafeModulePath {
        /// The manifest that holds the entry.
        path: PathBuf,
        /// The entry path the manifest declared.
        entry: String,
    },
    /// The manifest covers no module. A pass over it would report a
    /// clean tree and check nothing.
    ///
    /// The schema permits it: a PUP whose modules all failed to
    /// decrypt installs this way.
    #[error(
        "{} covers no module, so verifying against it would pass without checking anything; \
         reinstall the version with `cellgov firmware install <PS3UPDAT.PUP>` under a vault \
         that opens its modules",
        path.display()
    )]
    EmptyManifest {
        /// The manifest that covers nothing.
        path: PathBuf,
    },
    /// This build cannot open the SCE wrappers the manifest hashes
    /// through.
    #[error(
        "verifying a firmware tree decrypts every module {MANIFEST_FILE} names, and this \
         cellgov was built without the `decrypt` cargo feature; rebuild with \
         `cargo build -p cellgov_cli --features decrypt`"
    )]
    DecryptFeatureDisabled,
}

/// Read the `firmware.toml` covering `dev_flash_dir`.
///
/// Every `[[files]]` path it returns is a name that stays inside the
/// mount, so a caller may join one onto `dev_flash_dir`.
///
/// # Errors
///
/// - [`FirmwareVerifyError::NoManifest`] when the mount holds none.
/// - The read and parse refusals for a manifest that is there.
/// - [`FirmwareVerifyError::UnsafeModulePath`] for an entry path that
///   would leave the mount.
pub fn load_manifest(dev_flash_dir: &Path) -> Result<FirmwareManifest, FirmwareVerifyError> {
    let path = dev_flash_dir.join(MANIFEST_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FirmwareVerifyError::NoManifest {
                dir: dev_flash_dir.to_path_buf(),
            })
        }
        Err(source) => {
            return Err(FirmwareVerifyError::ManifestRead {
                path: path.clone(),
                source,
            })
        }
    };
    let firmware =
        manifest::parse_manifest(&text).map_err(|source| FirmwareVerifyError::ManifestParse {
            path: path.clone(),
            source: Box::new(source),
        })?;
    // The pass joins a `[[files]]` path onto the mount the way a
    // record's `[files]` key joins onto its tree, so both take the same
    // gate. Each of these aims the read outside the mount:
    //
    // - `..`, or an empty segment
    // - a `\` Win32 reads as a separator
    // - a `:` Win32 reads as a drive marker
    for entry in &firmware.files {
        if !crate::store::record::tree_rel_path_is_safe(&entry.path) {
            return Err(FirmwareVerifyError::UnsafeModulePath {
                path: path.clone(),
                entry: entry.path.clone(),
            });
        }
    }
    Ok(firmware)
}

/// Re-hash every module `firmware.toml` names under `dev_flash_dir`.
///
/// # Errors
///
/// Always [`FirmwareVerifyError::DecryptFeatureDisabled`] in this
/// build.
#[cfg(not(feature = "decrypt"))]
pub fn verify_firmware_tree(
    _dev_flash_dir: &Path,
    _keys: &KeyVault,
) -> Result<FirmwareVerifyReport, FirmwareVerifyError> {
    Err(FirmwareVerifyError::DecryptFeatureDisabled)
}

/// Whether a decrypt refusal is an answer about the vault rather than
/// about the module it was reading.
///
/// None of these refusals lets the pass rule on the tree, so none may
/// become a divergence.
#[cfg(feature = "decrypt")]
fn is_vault_gap(e: &crate::sce::SceError) -> bool {
    use crate::sce::SceError;

    matches!(
        e,
        SceError::Keys(_)
            | SceError::NoAppKey { .. }
            | SceError::NoNpdrmKey { .. }
            | SceError::NoCandidateOpensEnvelope { .. }
            | SceError::NoRapForNpdrmTitle { .. }
            | SceError::RapPboxNotAPermutation { .. }
    )
}

/// Re-hash every module `firmware.toml` names under `dev_flash_dir`.
///
/// The pass covers the manifest, and a clean report speaks for no more
/// than that. The installer records only the `.prx` / `.sprx` files it
/// could decrypt, so the pass reads none of these:
///
/// - every `.self` executable and every non-module file
/// - the modules the vault that installed them could not open
/// - anything added to the tree after the install
///
/// # Errors
///
/// - Every [`load_manifest`] refusal.
/// - [`FirmwareVerifyError::EmptyManifest`] for a manifest that covers
///   nothing.
/// - [`FirmwareVerifyError::ModuleRead`] for a module that can be
///   neither read nor shown absent.
/// - [`FirmwareVerifyError::ModuleKeyMissing`] when the vault cannot
///   open a module.
#[cfg(feature = "decrypt")]
pub fn verify_firmware_tree(
    dev_flash_dir: &Path,
    keys: &KeyVault,
) -> Result<FirmwareVerifyReport, FirmwareVerifyError> {
    use cellgov_ps3_abi::elf::ELF_MAGIC;

    let firmware = load_manifest(dev_flash_dir)?;
    if firmware.files.is_empty() {
        return Err(FirmwareVerifyError::EmptyManifest {
            path: dev_flash_dir.join(MANIFEST_FILE),
        });
    }
    let mut report = FirmwareVerifyReport::default();
    for entry in &firmware.files {
        let path = entry
            .path
            .split('/')
            .fold(dev_flash_dir.to_path_buf(), |dir, part| dir.join(part));
        let raw = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                report.divergences.push(ModuleFault {
                    path,
                    kind: ModuleDivergence::Missing,
                });
                continue;
            }
            Err(source) => return Err(FirmwareVerifyError::ModuleRead { path, source }),
        };
        let image = if crate::self_image::is_sce_wrapped(&raw) {
            match crate::sce::decrypt_self_to_elf(&raw, keys) {
                Ok(elf) => elf,
                // A vault short of the key that installed this entry
                // cannot tell an intact module from a replaced one.
                Err(source) if is_vault_gap(&source) => {
                    return Err(FirmwareVerifyError::ModuleKeyMissing {
                        path,
                        source: Box::new(source),
                    })
                }
                Err(source) => {
                    report.divergences.push(ModuleFault {
                        path,
                        kind: ModuleDivergence::NoImage {
                            reason: source.to_string(),
                        },
                    });
                    continue;
                }
            }
        } else if raw.starts_with(&ELF_MAGIC) {
            // A pre-decrypted `.prx` is its own post-decrypt image, as
            // it was when the manifest recorded it.
            raw
        } else {
            report.divergences.push(ModuleFault {
                path,
                kind: ModuleDivergence::NoImage {
                    reason: format!("{} bytes, neither an SCE container nor an ELF", raw.len()),
                },
            });
            continue;
        };
        let found = manifest::Sha256(manifest::sha256_of(&image));
        if found == entry.sha256 {
            report.matched += 1;
        } else {
            report.divergences.push(ModuleFault {
                path,
                kind: ModuleDivergence::Modified {
                    expected: entry.sha256,
                    found,
                },
            });
        }
    }
    Ok(report)
}

#[cfg(test)]
#[path = "tests/firmware_verify_tests.rs"]
mod tests;
