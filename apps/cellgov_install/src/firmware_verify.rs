//! Hold an installed firmware tree against the `firmware.toml` inside
//! it.
//!
//! The manifest records each module's post-decrypt hash, so the check
//! decrypts every entry the manifest names. A build without the
//! `decrypt` feature cannot open the SCE wrappers, so it cannot run
//! this check.

use std::path::{Path, PathBuf};

use crate::keys::KeyVault;
use crate::manifest::{self, FirmwareManifest, ManifestError, Sha256 as HexSha256, MANIFEST_FILE};
use crate::store::record::{stored_kernel, CoreOsRecord, KernelAbsence, KernelRecord};
use cellgov_ps3_abi::format::dev_flash::FLASH_MOUNT;

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
            | SceError::NoLv2Key { .. }
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
    let firmware = load_manifest(dev_flash_dir)?;
    if firmware.files.is_empty() {
        return Err(FirmwareVerifyError::EmptyManifest {
            path: dev_flash_dir.join(MANIFEST_FILE),
        });
    }
    let mut report = FirmwareVerifyReport::default();
    for module in module_images(dev_flash_dir, firmware.files, keys) {
        let module = module?;
        match module.image {
            Ok(_) => report.matched += 1,
            Err(kind) => report.divergences.push(ModuleFault {
                path: module.path,
                kind,
            }),
        }
    }
    Ok(report)
}

/// One manifest entry's module, read, opened and held to its hash.
#[cfg(feature = "decrypt")]
#[derive(Debug)]
pub struct ModuleImage {
    /// The entry path the manifest names, `/`-separated under the mount.
    pub entry: String,
    /// Where the module lies on disk.
    pub path: PathBuf,
    /// The post-decrypt image, when it hashes to the manifest's digest,
    /// or how the module diverged.
    pub image: Result<Vec<u8>, ModuleDivergence>,
}

/// The modules of a firmware mount, one [`ModuleImage`] per manifest
/// entry, in the order the caller gives the entries. [`module_images`]
/// builds it.
#[cfg(feature = "decrypt")]
#[derive(Debug)]
#[must_use = "the walk reads nothing until it is iterated"]
pub struct ModuleImages<'a> {
    dev_flash_dir: &'a Path,
    keys: &'a KeyVault,
    entries: std::vec::IntoIter<manifest::FirmwareFileEntry>,
}

/// Open each module `entries` names under `dev_flash_dir`.
///
/// The walk decrypts an SCE-wrapped module under `keys`; a plain ELF is
/// its own post-decrypt image, as it was when the manifest recorded it.
///
/// Each item is an error when an entry path would leave the mount
/// ([`FirmwareVerifyError::UnsafeModulePath`]), when the module can be
/// neither read nor shown absent ([`FirmwareVerifyError::ModuleRead`]),
/// or when the vault cannot open it
/// ([`FirmwareVerifyError::ModuleKeyMissing`]).
#[cfg(feature = "decrypt")]
pub fn module_images<'a>(
    dev_flash_dir: &'a Path,
    entries: Vec<manifest::FirmwareFileEntry>,
    keys: &'a KeyVault,
) -> ModuleImages<'a> {
    ModuleImages {
        dev_flash_dir,
        keys,
        entries: entries.into_iter(),
    }
}

#[cfg(feature = "decrypt")]
impl Iterator for ModuleImages<'_> {
    type Item = Result<ModuleImage, FirmwareVerifyError>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        if !crate::store::record::tree_rel_path_is_safe(&entry.path) {
            return Some(Err(FirmwareVerifyError::UnsafeModulePath {
                path: self.dev_flash_dir.join(MANIFEST_FILE),
                entry: entry.path,
            }));
        }
        let path = entry
            .path
            .split('/')
            .fold(self.dev_flash_dir.to_path_buf(), |dir, part| dir.join(part));
        let image = match open_module(&path, self.keys) {
            Ok(Ok(image)) => {
                let found = manifest::Sha256(manifest::sha256_of(&image));
                if found == entry.sha256 {
                    Ok(image)
                } else {
                    Err(ModuleDivergence::Modified {
                        expected: entry.sha256,
                        found,
                    })
                }
            }
            Ok(Err(kind)) => Err(kind),
            Err(error) => return Some(Err(error)),
        };
        Some(Ok(ModuleImage {
            entry: entry.path,
            path,
            image,
        }))
    }
}

/// The post-decrypt image of the module at `path`, or why it yields
/// none.
#[cfg(feature = "decrypt")]
fn open_module(
    path: &Path,
    keys: &KeyVault,
) -> Result<Result<Vec<u8>, ModuleDivergence>, FirmwareVerifyError> {
    use cellgov_ps3_abi::format::elf::ELF_MAGIC;

    let raw = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Err(ModuleDivergence::Missing))
        }
        Err(source) => {
            return Err(FirmwareVerifyError::ModuleRead {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    if crate::self_image::is_sce_wrapped(&raw) {
        return match crate::sce::decrypt_self_to_elf(&raw, keys) {
            Ok(elf) => Ok(Ok(elf)),
            // A vault short of the key that installed this entry
            // cannot tell an intact module from a replaced one.
            Err(source) if is_vault_gap(&source) => Err(FirmwareVerifyError::ModuleKeyMissing {
                path: path.to_path_buf(),
                source: Box::new(source),
            }),
            Err(source) => Ok(Err(ModuleDivergence::NoImage {
                reason: source.to_string(),
            })),
        };
    }
    if raw.starts_with(&ELF_MAGIC) {
        return Ok(Ok(raw));
    }
    Ok(Err(ModuleDivergence::NoImage {
        reason: format!("{} bytes, neither an SCE container nor an ELF", raw.len()),
    }))
}

/// Re-hash the stored kernel `kernel` names under `entry_dir`.
///
/// The record holds the as-stored digest, so this hashes the bytes as
/// they lie and needs no key. `None` is a match.
///
/// # Errors
///
/// [`FirmwareVerifyError::ModuleRead`] when the read fails with
/// anything other than `NotFound`.
pub fn verify_stored_kernel(
    entry_dir: &Path,
    kernel: &KernelRecord,
) -> Result<Option<ModuleFault>, FirmwareVerifyError> {
    let path = kernel.path_in(entry_dir);
    let raw = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(ModuleFault {
                path,
                kind: ModuleDivergence::Missing,
            }))
        }
        Err(source) => return Err(FirmwareVerifyError::ModuleRead { path, source }),
    };
    let found = manifest::Sha256(manifest::sha256_of(&raw));
    if found == kernel.stored_sha256 {
        return Ok(None);
    }
    Ok(Some(ModuleFault {
        path,
        kind: ModuleDivergence::Modified {
            expected: kernel.stored_sha256,
            found,
        },
    }))
}

/// One firmware entry held against its manifest and its record.
#[derive(Debug)]
pub struct EntryVerifyReport<'a> {
    /// The modules, and the stored kernel when the entry holds one.
    pub report: FirmwareVerifyReport,
    /// Why the pass checked no kernel, when the entry stores none.
    pub kernel_absence: Option<KernelAbsence<'a>>,
}

/// Holds the firmware entry at `entry_dir` against its record: every
/// module the entry's `firmware.toml` lists, and the kernel its
/// `[core_os]` block names. `core_os` is `None` for a record that
/// predates the block.
///
/// # Errors
///
/// Every [`FirmwareVerifyError`] of [`verify_firmware_tree`] and
/// [`verify_stored_kernel`].
pub fn verify_firmware_entry<'a>(
    entry_dir: &Path,
    core_os: Option<&'a CoreOsRecord>,
    keys: &KeyVault,
) -> Result<EntryVerifyReport<'a>, FirmwareVerifyError> {
    let mut report = verify_firmware_tree(&entry_dir.join(FLASH_MOUNT), keys)?;
    let kernel_absence = match stored_kernel(core_os) {
        Ok(kernel) => {
            match verify_stored_kernel(entry_dir, kernel)? {
                None => report.matched += 1,
                Some(fault) => report.divergences.push(fault),
            }
            None
        }
        Err(absence) => Some(absence),
    };
    Ok(EntryVerifyReport {
        report,
        kernel_absence,
    })
}

#[cfg(test)]
#[path = "tests/firmware_verify_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/verify_stored_kernel_tests.rs"]
mod verify_stored_kernel_tests;
