//! The failure type for a firmware install.

use std::path::PathBuf;

use cellgov_ps3_abi::format::pup::ENTRY_ID_UPDATE_FILES;

use crate::manifest::{ManifestError, Sha256 as HexSha256};
use crate::sce::SceError;
use crate::store::rename::RenameRefused;
use crate::tar::{ExtractError, TarParseError};

/// Why one dev_flash package produced no files.
#[derive(Debug, thiserror::Error)]
pub enum PackageFailure {
    /// The SCE envelope did not open under any package keyset.
    #[error("{package}: {source}")]
    Decrypt {
        /// Outer-TAR name of the package.
        package: String,
        /// Why the decrypt failed.
        #[source]
        source: SceError,
    },
    /// The decrypted payload is not the TAR the package should carry.
    #[error("{package}: inner TAR parse: {source}")]
    InnerTar {
        /// Outer-TAR name of the package.
        package: String,
        /// Why the payload would not parse.
        #[source]
        source: TarParseError,
    },
}

/// Why a firmware install failed.
#[derive(Debug, thiserror::Error)]
pub enum FirmwareInstallError {
    /// PUP parsing or HMAC validation failed.
    #[error("PUP: {0}")]
    Pup(#[from] crate::pup::PupError),
    /// The PUP carries no `update_files` entry, so it holds no
    /// firmware payload.
    #[error("PUP has no entry 0x{ENTRY_ID_UPDATE_FILES:x} (update_files)")]
    NoUpdateFiles,
    /// The `update_files` entry is there, but the extent it declares
    /// leaves the file, so the payload it names is not in the buffer.
    #[error(
        "PUP entry 0x{ENTRY_ID_UPDATE_FILES:x} (update_files) spans \
         0x{offset:x}..+0x{length:x}, past the 0x{file_len:x}-byte file"
    )]
    UpdateFilesOutOfBounds {
        /// `data_offset` the entry declared.
        offset: u64,
        /// `data_length` the entry declared.
        length: u64,
        /// Length of the PUP buffer the extent was measured against.
        file_len: usize,
    },
    /// The `update_files` payload is not a parseable TAR.
    #[error("PUP update_files TAR: {0}")]
    OuterTar(#[source] TarParseError),
    /// The `update_files` TAR carries no dev_flash payload package, so
    /// there is no firmware tree to extract.
    #[error("PUP update_files carries no dev_flash_* package")]
    NoDevFlashPackages,
    /// Every dev_flash package was empty or pruned away, so the
    /// extraction wrote nothing to claim success over.
    #[error("install produced 0 files from {packages} dev_flash package(s)")]
    ProducedNothing {
        /// Number of dev_flash packages the outer TAR carried.
        packages: usize,
    },
    /// A package would not decrypt, or an entry would not be written,
    /// so the staged tree is short of the firmware the PUP carries.
    #[error(
        "partial install: {files} file(s) staged, {} of {packages} package(s) failed, \
         {} entry write(s) failed",
        packages_failed.len(), extract_errors.len()
    )]
    PartialInstall {
        /// Files that did land in the staging tree.
        files: usize,
        /// Number of dev_flash packages attempted.
        packages: usize,
        /// The packages that produced nothing, in encounter order.
        packages_failed: Vec<PackageFailure>,
        /// Per-entry write failures, in encounter order.
        extract_errors: Vec<ExtractError>,
    },
    /// `vsh/etc/version.txt` is absent from the extracted tree or could
    /// not be read, so nothing names the entry the install commits to.
    #[error("read {}: {source}", path.display())]
    VersionUnreadable {
        /// Where the version file was expected.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// `version.txt` is there but carries no `<field>:<version>:`
    /// record, so no version can be read out of it.
    #[error("{} carries no colon-delimited version field", path.display())]
    VersionUnparseable {
        /// The file that was read.
        path: PathBuf,
    },
    /// That firmware version is installed, from this same PUP.
    #[error(
        "firmware {version} is already installed, from this same PUP (sha256 {}); \
         pass --force to reinstall it",
        pup_sha256.to_hex()
    )]
    VersionInstalled {
        /// The version key that is already taken.
        version: String,
        /// The source hash both installs share.
        pup_sha256: HexSha256,
    },
    /// That firmware version is installed, from a different PUP.
    #[error(
        "firmware {version} is already installed, from PUP sha256 {}, and this PUP is {}; \
         pass --force to replace it",
        installed.to_hex(), incoming.to_hex()
    )]
    VersionInstalledFromAnotherPup {
        /// The version key that is already taken.
        version: String,
        /// Source hash the installed entry came from.
        installed: HexSha256,
        /// Source hash of the PUP being installed.
        incoming: HexSha256,
    },
    /// The entry directory holds an unrecorded tree, so the store
    /// cannot say what is in it.
    #[error("install target {} already exists; pass --force to overwrite", path.display())]
    TargetExists {
        /// The non-empty entry directory.
        path: PathBuf,
    },
    /// An install record that is present but this build will not read.
    #[error("install record {}: {source}", path.display())]
    RecordParse {
        /// The record that would not parse.
        path: PathBuf,
        /// Why the record was refused.
        #[source]
        source: Box<crate::store::record::InstallRecordParseError>,
    },
    /// A record naming this firmware entry describes something else.
    #[error(
        "install record {} does not describe firmware {version:?}",
        path.display()
    )]
    RecordMismatch {
        /// The offending record.
        path: PathBuf,
        /// The version being installed.
        version: String,
    },
    /// Building or serialising `firmware.toml` failed.
    #[error("{0}")]
    Manifest(#[from] ManifestError),
    /// Decrypting an installed module to hash it for the manifest
    /// failed for a reason other than a missing key.
    #[error("read {}: {source}", path.display())]
    ModuleReadFailed {
        /// The module that could not be read.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A module's path under the firmware tree is not valid UTF-8, so
    /// `firmware.toml` cannot name it.
    #[error("non-utf8 firmware path: {}", path.display())]
    NonUtf8Path {
        /// The offending path.
        path: PathBuf,
    },
    /// The commit rename failed, so the entry was never created.
    #[error(
        "commit {} -> {}: {source}; the extracted tree is left at {} and the next install \
         reuses or sweeps it, so retrying is safe",
        staging_root.display(), entry_dir.display(), staging_root.display()
    )]
    CommitFailed {
        /// The staged tree still on disk.
        staging_root: PathBuf,
        /// The entry it was to become.
        entry_dir: PathBuf,
        /// The refusal and its attempt count.
        #[source]
        source: RenameRefused,
    },
    /// A filesystem operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// The operation that failed (e.g. "write", "remove").
        op: &'static str,
        /// The path involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Serialising the install record failed.
    #[error("install-record serialise: {0}")]
    RecordSerialise(#[from] toml::ser::Error),
    /// Another writer holds this artifact.
    #[error("{0}")]
    Locked(#[from] crate::store::lock::StoreLockError),
    /// The pre-store check refused the root.
    #[error("{0}")]
    PreStore(#[from] crate::store::pre_store::PreStoreError),
    /// The version read from `version.txt` is not usable as a store
    /// directory name.
    #[error("store key: {0}")]
    StoreKey(#[from] crate::store::layout::StoreKeyError),
    /// The entry directory could not be expressed as a record
    /// `store_path` under the VFS root the record lives in.
    #[error("record store path: {0}")]
    StorePath(#[from] crate::store::layout::StorePathError),
    /// A pre-commit fault was followed by a cleanup that could not
    /// discard the staging root, so residue outlived the failed install.
    #[error("{cause}; the staging root {} could not be discarded: {source}", path.display())]
    StagingResidue {
        /// The staging root still on disk.
        path: PathBuf,
        /// Why the cleanup removal failed.
        #[source]
        source: std::io::Error,
        /// The pre-commit fault that triggered the cleanup.
        cause: Box<FirmwareInstallError>,
    },
}
