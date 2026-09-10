//! The failure type shared by the base and update installers.

use std::path::PathBuf;

use crate::iso;
use crate::manifest::Sha256 as HexSha256;
use crate::param_sfo;
use crate::pkg;
use crate::store::layout::ArtifactKind;
use crate::store::rename::RenameRefused;
use cellgov_ps3_abi::format::title_tree::DISC_UPDATE_PUP;

/// Why a game install failed.
#[derive(Debug, thiserror::Error)]
pub enum GameInstallError {
    /// PKG parsing / extraction failed.
    #[error("PKG: {0}")]
    Pkg(#[from] pkg::PkgError),
    /// PARAM.SFO parsing failed.
    #[error("PARAM.SFO: {0}")]
    Sfo(#[from] param_sfo::SfoError),
    /// ISO9660 reading failed.
    #[error("ISO: {0}")]
    Iso(#[from] iso::IsoError),
    /// Reading the EBOOT's NPD header failed.
    #[error("NPD header: {0}")]
    Npd(#[source] crate::sce::SceError),
    /// The target directory names no staging sibling to stage into.
    #[error("{0}")]
    HiddenSibling(#[from] crate::store::layout::HiddenSiblingError),
    /// The package carried no PARAM.SFO.
    #[error("PKG has no PARAM.SFO")]
    NoParamSfo,
    /// PARAM.SFO had no `TITLE_ID`.
    #[error("PARAM.SFO has no TITLE_ID")]
    MissingTitleId,
    /// Category is not `HG` (HDD game); this installer handles only
    /// PSN/retail HDD titles.
    #[error("PKG category {category:?} is not HG (HDD game)")]
    NotHddGame {
        /// The PARAM.SFO `CATEGORY` value found.
        category: String,
    },
    /// Disc category is not a disc-game category (`DG`/`GD`).
    #[error("ISO category {category:?} is not a disc game (DG/GD)")]
    NotDiscGame {
        /// The PARAM.SFO `CATEGORY` value found.
        category: String,
    },
    /// The disc carried no `PS3_GAME/PARAM.SFO`.
    #[error("ISO has no PS3_GAME/PARAM.SFO")]
    NoDiscParamSfo,
    /// The disc carried no `PS3_GAME/USRDIR/EBOOT.BIN`.
    #[error("ISO has no PS3_GAME/USRDIR/EBOOT.BIN")]
    NoDiscEboot,
    /// A file the disc tree names does not open with its format's
    /// magic: the image is still carrying its disc encryption.
    #[error(
        "ISO {path} opens with 0x{:02x}{:02x}{:02x}{:02x} rather than its format's magic; \
         `cellgov title install` takes a decrypted dump of a disc you own, and this \
         image reads as still encrypted",
        head[0], head[1], head[2], head[3]
    )]
    DiscImageEncrypted {
        /// Disc-relative path of the file that did not open.
        path: &'static str,
        /// Its first four bytes.
        head: [u8; 4],
    },
    /// Header content-id does not embed the PARAM.SFO `TITLE_ID`.
    #[error("title-id mismatch: header content-id {header:?} does not contain PARAM.SFO TITLE_ID {sfo:?}")]
    TitleIdMismatch {
        /// Header content-id field.
        header: String,
        /// PARAM.SFO `TITLE_ID`.
        sfo: String,
    },
    /// The EBOOT NPD content-id does not embed the title-id.
    #[error(
        "content-id mismatch: EBOOT NPD content-id {npd:?} does not contain title-id {title_id:?}"
    )]
    ContentIdMismatch {
        /// EBOOT NPD header content-id.
        npd: String,
        /// PARAM.SFO `TITLE_ID`.
        title_id: String,
    },
    /// The package carried no `USRDIR/EBOOT.BIN`.
    #[error("PKG has no USRDIR/EBOOT.BIN")]
    NoEboot,
    /// Category is neither `GD` (disc-title patch) nor `HG`
    /// (HDD-title patch), so the package is not an update.
    #[error("PKG category {category:?} is not an update package (GD/HG)")]
    NotUpdatePackage {
        /// The PARAM.SFO `CATEGORY` value found.
        category: String,
    },
    /// The update PARAM.SFO carries neither `APP_VER` nor `VERSION`.
    #[error("PARAM.SFO has no APP_VER or VERSION; an update version names its store directory")]
    MissingAppVersion,
    /// The base record already under this title's store entry names
    /// another title, or another kind of artifact.
    #[error(
        "base record {} declares a {} entry for title {found:?}, not a base for {expected:?}",
        path.display(), kind.as_str()
    )]
    BaseRecordMismatch {
        /// Where the offending base record is.
        path: PathBuf,
        /// The kind the record declares.
        kind: ArtifactKind,
        /// The title it names; empty when it carries no `[title]`.
        found: String,
        /// The title id of the update being installed.
        expected: String,
    },
    /// That update version is already installed.
    #[error(
        "update {version} is already installed, from source sha256 {}; \
         pass --force to replace it",
        existing_source.to_hex()
    )]
    UpdateVersionInstalled {
        /// The version key that is already taken.
        version: String,
        /// Source-container hash the installed version came from.
        existing_source: HexSha256,
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
    /// A license-1/2 (Network/Local) title was installed with no RAP.
    #[error("NPDRM title {content_id:?} requires a RAP (license 1/2); pass --rap")]
    RapRequired {
        /// Full NPD content id.
        content_id: String,
    },
    /// The supplied RAP was not exactly 16 bytes.
    #[error("RAP must be exactly 16 bytes, got {len}")]
    RapWrongSize {
        /// Observed RAP length.
        len: usize,
    },
    /// The decrypt-proof gate failed: the installed tree would not load.
    #[error("decrypt-proof failed (the installed EBOOT does not decrypt): {0}")]
    DecryptProof(#[source] crate::sce::SceError),
    /// The system software the disc ships did not register as a firmware
    /// entry. The install staged nothing of the title.
    #[error(
        "the disc's {DISC_UPDATE_PUP}: {source}; pass --no-firmware to install the title \
         without the firmware it ships"
    )]
    ShippedFirmware {
        /// The refusal, from the package check or the firmware installer.
        #[source]
        source: crate::firmware_install::FirmwareInstallError,
    },
    /// The disc holds a directory where its update package would be.
    #[error(
        "the disc's {DISC_UPDATE_PUP} is a directory, not an update package; pass --no-firmware \
         to install the title without the firmware it ships"
    )]
    ShippedFirmwareNotAFile,
    /// The disc's package names one version in its plaintext
    /// `version.txt` entry and another in the tree it unpacks to. The
    /// installer committed the tree as a firmware entry under its own
    /// version; the title did not install.
    #[error(
        "the disc's {DISC_UPDATE_PUP} names firmware {declared} in its version.txt entry, but the \
         tree it unpacks to names {extracted}; that tree is installed as firmware {extracted}, \
         and the title was not installed"
    )]
    ShippedFirmwareVersionMismatch {
        /// The version the package's `version.txt` entry spells.
        declared: String,
        /// The version the unpacked tree's `vsh/etc/version.txt` spells.
        extracted: String,
    },
    /// The install target already exists and `--force` was not set.
    #[error("install target {} already exists; pass --force to overwrite", path.display())]
    TargetExists {
        /// The non-empty target directory.
        path: PathBuf,
    },
    /// A commit rename stayed refused through every attempt it got.
    #[error("rename {}: {source}", path.display())]
    Rename {
        /// The target of the rename.
        path: PathBuf,
        /// The refusal and its attempt count.
        #[source]
        source: RenameRefused,
    },
    /// A filesystem operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// The operation, e.g. "write" or "remove".
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
    /// A staged entry's path carried an absolute, prefix, or `..`
    /// component and would escape the staging root.
    #[error("unsafe entry path {path:?}")]
    UnsafeEntryPath {
        /// The offending package/disc-relative path.
        path: String,
    },
    /// A content-id (game-directory key or RAP-filename base) carried a
    /// byte outside `[A-Za-z0-9._-]` and is unsafe as a path component.
    #[error("unsafe content-id {content_id:?}")]
    UnsafeContentId {
        /// The offending content-id.
        content_id: String,
    },
    /// The pre-store check refused the root.
    #[error("{0}")]
    PreStore(#[from] crate::store::pre_store::PreStoreError),
    /// Another writer holds this artifact.
    #[error("{0}")]
    Locked(#[from] crate::store::lock::StoreLockError),
    /// A store key -- the title id that names the store directory --
    /// is not usable as a directory name.
    #[error("store key: {0}")]
    StoreKey(#[from] crate::store::layout::StoreKeyError),
    /// The installed tree could not be expressed as a record
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
        cause: Box<GameInstallError>,
    },
}
