//! Game-install orchestration: turn a retail PKG or disc image into a
//! `dev_hdd0`/`dev_bdvd` tree, an installed RAP, and a
//! [`crate::store`] record naming all three.
//!
//! The pure container work lives in [`crate::pkg`] / [`crate::iso`];
//! this module owns the filesystem shell.
//!
//! # Invariants
//!
//! - The whole pre-commit batch -- game tree, staged RAP, and the
//!   decrypt-proof -- lives under one [`staging_sibling`] of the target
//!   directory; a fault before commit discards it whole.
//! - Commit runs only after the decrypt-proof passes, as a fixed
//!   rename sequence: the staged RAP into `exdata/` first, then the
//!   game tree into its final directory (the commit point), then the
//!   record last. The single residue window is between those two
//!   renames -- a tree-rename failure leaves an inert, content-id-keyed
//!   RAP with no game directory, read only when that title's EBOOT is
//!   decrypted and overwritten identically on retry.
//!
//! Both installers run the decrypt-proof, so they exist only with the
//! `decrypt` feature; the record types, the store layout, and the
//! uninstall side stay available in every build.

#![cfg_attr(
    not(feature = "decrypt"),
    allow(
        dead_code,
        unused_imports,
        reason = "the staging helpers are reachable only from the gated installers; the feature-on build lints them"
    )
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::iso;
use crate::keys::KeyVault;
use crate::manifest::Sha256 as HexSha256;
use crate::npdrm::{self, NpdHeaderInfo, NpdLicense};
use crate::param_sfo;
use crate::pkg::{self, PkgEntryKind};
use crate::progress::{Phase, ProgressSink};
use crate::sce;
use crate::self_image::is_sce_wrapped;
use crate::store::layout::{is_safe_component, staging_sibling, Artifact, StoreLayout, TitleId};
use crate::store::record::{
    tree_rel_path_is_safe, ArtifactRecord, InstallRecord, RapRecord, SourceRecord, TitleRecord,
    INSTALL_RECORD_FORMAT_VERSION,
};
use cellgov_ps3_abi::elf::ELF_MAGIC;

/// Knobs shared by `install_pkg` and `install_iso`.
#[derive(Clone, Copy)]
pub struct InstallOptions<'a> {
    /// Overwrite a non-empty target directory.
    pub force: bool,
    /// Where progress events go; `&()` drops them.
    pub progress: &'a dyn ProgressSink,
}

impl Default for InstallOptions<'_> {
    fn default() -> Self {
        Self {
            force: false,
            progress: &(),
        }
    }
}

/// PARAM.SFO categories that mark a disc title (`DG` disc game,
/// `GD` disc game/data).
const DISC_CATEGORIES: [&str; 2] = ["DG", "GD"];

/// A RAP staged under the staging root, ready to be renamed into
/// `exdata/` at commit and recorded in the install record.
struct StagedRap {
    /// `staging_root/rap/<content-id>.rap`.
    staged_path: PathBuf,
    /// `exdata/<content-id>.rap` -- the commit destination.
    final_path: PathBuf,
    record: RapRecord,
}

/// One file or directory queued for staging, normalized across the PKG
/// and ISO inputs so the commit machinery is container-agnostic.
struct StagedFile<'a> {
    /// Target-relative path, `/`-separated.
    path: String,
    is_dir: bool,
    data: StagedData<'a>,
}

/// Where a staged entry's bytes come from. [`stage_tree`] streams them
/// to disk one entry at a time, so neither variant requires the whole
/// tree resident -- a BD-DL disc's content exceeds host memory.
enum StagedData<'a> {
    /// One borrowed buffer (PKG entries; empty for a directory).
    Bytes(&'a [u8]),
    /// Ordered extent slices into the source disc image (ISO entries),
    /// bounds-checked at carve time.
    Slices(Vec<&'a [u8]>),
}

/// What a completed install produced, for the CLI to report.
#[derive(Debug, Clone)]
pub struct GameInstallOutcome {
    /// PARAM.SFO `TITLE_ID` (the game-directory key).
    pub title_id: String,
    /// Full NPD content id (the RAP-filename key), or the title-id for
    /// an APP-keyed title with no NPD header.
    pub content_id: String,
    /// The committed game-tree directory.
    pub game_dir: PathBuf,
    /// Whether a RAP was installed into `exdata/`.
    pub rap_installed: bool,
    /// Number of distinct files the committed tree holds, equal to the
    /// record's file count. Two container entries whose paths normalize
    /// to the same key are one file on disk and one record key, so the
    /// staged-entry count would over-report both.
    pub file_count: usize,
    /// The written install record.
    pub record_path: PathBuf,
}

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
         install-iso takes a decrypted dump of a disc you own, and this image reads as still encrypted",
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
    /// The game directory already exists and `--force` was not set.
    #[error("game directory {} already exists; pass --force to overwrite", path.display())]
    TargetExists {
        /// The non-empty target directory.
        path: PathBuf,
    },
    /// A filesystem operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// What was being attempted (e.g. "write", "rename").
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

pub(crate) fn sha256_of(bytes: &[u8]) -> HexSha256 {
    let mut h = Sha256::new();
    h.update(bytes);
    HexSha256(h.finalize().into())
}

fn io_err<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl Fn(std::io::Error) -> GameInstallError + 'a {
    move |source| GameInstallError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

/// Bytes written (and hashed) per progress event: large enough that
/// the reporter call is noise, small enough that a bar moves smoothly
/// through a multi-gigabyte extent.
const PROGRESS_PIECE: usize = 1 << 20;

/// Write `chunks` to `path` in order, creating parents, hashing the
/// stream as it is written, and `fsync` the file so the content is
/// durable before the commit rename. Chunks are borrowed views (of a
/// container buffer or a mapped image) split into [`PROGRESS_PIECE`]
/// sub-slices -- a borrow-only split, so residence stays bounded by
/// the caller's chunks.
fn write_chunks_and_sync<'c>(
    path: &Path,
    chunks: impl IntoIterator<Item = &'c [u8]>,
    progress: &dyn ProgressSink,
) -> Result<HexSha256, GameInstallError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
    }
    let mut f = std::fs::File::create(path).map_err(io_err("create", path))?;
    let mut hasher = Sha256::new();
    {
        use std::io::Write;
        for chunk in chunks {
            for piece in chunk.chunks(PROGRESS_PIECE) {
                hasher.update(piece);
                f.write_all(piece).map_err(io_err("write", path))?;
                progress.advanced(piece.len() as u64);
            }
        }
    }
    f.sync_all().map_err(io_err("fsync", path))?;
    Ok(HexSha256(hasher.finalize().into()))
}

/// Write `bytes` to `path`, creating parents, and `fsync` the file so
/// the content is durable before the commit rename.
fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<(), GameInstallError> {
    write_chunks_and_sync(path, std::iter::once(bytes), &()).map(|_| ())
}

/// Report the staging denominator: non-directory entry count and their
/// total bytes. Staged entries whose paths collide still each count,
/// so the totals can slightly exceed what lands; a renderer treats
/// them as a ceiling.
fn emit_totals(progress: &dyn ProgressSink, staged: &[StagedFile<'_>]) {
    let mut files = 0usize;
    let mut bytes = 0u64;
    for f in staged {
        if f.is_dir {
            continue;
        }
        files += 1;
        bytes += match &f.data {
            StagedData::Bytes(b) => b.len() as u64,
            StagedData::Slices(s) => s.iter().map(|c| c.len() as u64).sum(),
        };
    }
    progress.totals(files, bytes);
}

/// Join a container-relative entry path under `base`, rejecting any
/// component that could escape it.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, GameInstallError> {
    use std::path::Component::{CurDir, Normal, ParentDir, Prefix, RootDir};
    let mut out = base.to_path_buf();
    for comp in Path::new(rel).components() {
        match comp {
            Normal(c) => out.push(c),
            CurDir => {}
            RootDir | Prefix(_) | ParentDir => {
                return Err(GameInstallError::UnsafeEntryPath {
                    path: rel.to_string(),
                })
            }
        }
    }
    // The same entry becomes a record key, and the record gate refuses
    // a key it could not resolve back onto the tree.
    if !tree_rel_path_is_safe(&normalized_rel(rel)) {
        return Err(GameInstallError::UnsafeEntryPath {
            path: rel.to_string(),
        });
    }
    Ok(out)
}

/// The staging-relative path a staged entry lands at, as a
/// `/`-separated string built from the same `Normal` components
/// [`safe_join`] keeps, so the record key equals the on-disk path.
///
/// `Path::components` splits on the host's separators, so a container
/// entry carrying `\` or `:` normalizes to two keys on Win32 and one
/// on a POSIX host; [`safe_join`] refuses those names.
fn normalized_rel(rel: &str) -> String {
    Path::new(rel)
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Reject a content-id [`is_safe_component`] refuses, before it is used
/// to build a game-directory or RAP-file path.
fn validate_content_id(id: &str) -> Result<(), GameInstallError> {
    if is_safe_component(id) {
        Ok(())
    } else {
        Err(GameInstallError::UnsafeContentId {
            content_id: id.to_string(),
        })
    }
}

/// Whether a license consumes a RAP. Only network/local do; free and
/// APP-keyed (no NPD header) titles resolve their klicensee without one
/// (free falls back to the vault's free klicensee).
fn rap_consumed(license: Option<NpdLicense>) -> bool {
    match license {
        Some(NpdLicense::Network) | Some(NpdLicense::Local) => true,
        Some(NpdLicense::Free) | None => false,
    }
}

/// The RAP to stage for a title, or `None` for disc/free/no-RAP titles.
///
/// A RAP supplied for a title [`rap_consumed`] rejects is dropped here
/// and never staged, committed, or recorded.
fn plan_staged_rap(
    rap_needed: bool,
    rap: Option<&[u8]>,
    content_id: &str,
    staging_root: &Path,
    exdata: &Path,
) -> Option<StagedRap> {
    if !rap_needed {
        return None;
    }
    let r = rap.expect("invariant: rap_needed without a RAP returns RapRequired above");
    let filename = format!("{content_id}.rap");
    Some(StagedRap {
        staged_path: staging_root.join("rap").join(&filename),
        final_path: exdata.join(&filename),
        record: RapRecord {
            filename,
            sha256: sha256_of(r),
        },
    })
}

/// Whether `path` exists and contains at least one entry.
fn dir_non_empty(path: &Path) -> Result<bool, GameInstallError> {
    if !path.exists() {
        return Ok(false);
    }
    let mut entries = std::fs::read_dir(path).map_err(io_err("read dir", path))?;
    Ok(entries.next().is_some())
}

/// Install a retail PKG (PSN/retail HDD title) into `output_dir`'s
/// `dev_hdd0` tree, installing the RAP and writing the title-base
/// record for that root.
#[cfg(feature = "decrypt")]
pub fn install_pkg(
    pkg_bytes: &[u8],
    rap: Option<&[u8]>,
    keys: &KeyVault,
    output_dir: &Path,
    opts: InstallOptions<'_>,
) -> Result<GameInstallOutcome, GameInstallError> {
    let progress = opts.progress;
    progress.phase(Phase::Reading.code());
    let archive = pkg::extract(pkg_bytes, keys)?;

    // PARAM.SFO -> identity + HDD-game gate.
    let sfo_file = archive
        .files
        .iter()
        .find(|f| f.name == "PARAM.SFO")
        .ok_or(GameInstallError::NoParamSfo)?;
    let (title_id, category, title, app_version) = parse_identity(archive.file_data(sfo_file))?;
    if category != "HG" {
        return Err(GameInstallError::NotHddGame { category });
    }
    // The header's content-id field embeds the 9-char title-id (e.g.
    // `UP9000-NPUA80001_00-...` carries `NPUA80001`), so a containment
    // check is robust to the prefix/suffix the SFO TITLE_ID omits.
    if !archive.header.content_id.is_empty() && !archive.header.content_id.contains(&title_id) {
        return Err(GameInstallError::TitleIdMismatch {
            header: archive.header.content_id.clone(),
            sfo: title_id,
        });
    }

    // EBOOT -> license type + full content id.
    let eboot = archive
        .files
        .iter()
        .find(|f| f.name == "USRDIR/EBOOT.BIN")
        .ok_or(GameInstallError::NoEboot)?;
    let eboot_data = archive.file_data(eboot);
    let npd = npdrm::find_npd_header_info(eboot_data).map_err(GameInstallError::Npd)?;
    // The EBOOT's NPD content-id must also embed the title-id, tying
    // the executable's own identity to the PARAM.SFO / header.
    if let Some(n) = &npd {
        if !n.content_id.contains(&title_id) {
            return Err(GameInstallError::ContentIdMismatch {
                npd: n.content_id.clone(),
                title_id,
            });
        }
    }
    let content_id = npd
        .as_ref()
        .map(|n| n.content_id.clone())
        .unwrap_or_else(|| title_id.clone());

    // Both keys become path components (game/<title-id>,
    // exdata/<content-id>.rap); reject anything unsafe before use.
    validate_content_id(&title_id)?;
    validate_content_id(&content_id)?;

    // Layout. The pre-commit batch lives under one staging root:
    // `tree/` is the game tree, `rap/` holds the staged RAP.
    let layout = StoreLayout::new(output_dir);
    let exdata = layout.live_exdata_dir();
    let final_dir = output_dir.join("dev_hdd0").join("game").join(&title_id);
    let staging_root = staging_sibling(&final_dir);
    let tree_staging = staging_root.join("tree");
    // Resolved before staging: only `run_or_clean` discards the staging
    // root, so a fallible step between it and `commit` would leave the
    // staged tree behind.
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(&title_id)?,
    };
    let store_path = layout.store_path_of(&final_dir)?;

    if dir_non_empty(&final_dir)? && !opts.force {
        return Err(GameInstallError::TargetExists { path: final_dir });
    }

    // Fail fast on the RAP contract before staging anything.
    let license = npd.as_ref().map(|n| n.license);
    let rap_needed = rap_consumed(license);
    if rap_needed && rap.is_none() {
        return Err(GameInstallError::RapRequired { content_id });
    }
    if let Some(r) = rap {
        if r.len() != 16 {
            return Err(GameInstallError::RapWrongSize { len: r.len() });
        }
    }

    let staged: Vec<StagedFile> = archive
        .files
        .iter()
        .map(|f| StagedFile {
            path: f.name.clone(),
            is_dir: f.kind == PkgEntryKind::Directory,
            data: StagedData::Bytes(archive.file_data(f)),
        })
        .collect();
    emit_totals(progress, &staged);

    // Build the whole batch (tree + RAP + proof) under the staging root.
    prepare_staging(&staging_root, progress)?;
    let (file_digests, staged_rap) = run_or_clean(&staging_root, || {
        progress.phase(Phase::Staging.code());
        let file_digests = stage_tree(&staged, &tree_staging, progress)?;
        let staged_rap = plan_staged_rap(rap_needed, rap, &content_id, &staging_root, &exdata);
        if let Some(sr) = &staged_rap {
            let rap_bytes = rap.expect(
                "invariant: plan_staged_rap returns Some only when rap_needed, so rap is set",
            );
            write_and_sync(&sr.staged_path, rap_bytes)?;
        }
        // The proof resolves the RAP the way the load path will -- a
        // content-id-keyed read -- but against the staged copy, so a
        // fault before commit touches no live exdata.
        let rap_staging = staging_root.join("rap");
        let resolver = move |n: &NpdHeaderInfo| -> Option<npdrm::Rap> {
            let rap_path = rap_staging.join(format!("{}.rap", n.content_id));
            let bytes = std::fs::read(&rap_path).ok()?;
            let arr: [u8; 16] = bytes.as_slice().try_into().ok()?;
            Some(npdrm::Rap(arr))
        };
        progress.phase(Phase::Proving.code());
        npdrm::decrypt_self_to_elf_auto(eboot_data, keys, resolver)
            .map_err(GameInstallError::DecryptProof)?;
        Ok((file_digests, staged_rap))
    })?;

    let rap_installed = staged_rap.is_some();
    // The source hash is a full read of the container: its own phase,
    // or a multi-gigabyte container's hash time hides under the proof
    // label.
    progress.phase(Phase::Hashing.code());
    let record = build_record(
        "pkg",
        pkg_bytes,
        ArtifactRecord {
            kind: artifact.kind(),
            version: app_version,
            store_path,
        },
        file_digests,
        TitleRecord {
            title_id: title_id.clone(),
            content_id: content_id.clone(),
            category,
            title,
            distribution: "psn-hdd".to_string(),
        },
        staged_rap.as_ref().map(|r| r.record.clone()),
    );
    let record_path = commit(
        &staging_root,
        &tree_staging,
        &final_dir,
        staged_rap.as_ref(),
        &layout.record_path(&artifact),
        &record,
        progress,
    )?;

    progress.finished();
    Ok(GameInstallOutcome {
        title_id,
        content_id,
        game_dir: final_dir,
        rap_installed,
        file_count: record.files.len(),
        record_path,
    })
}

/// Install a decrypted disc image's `PS3_GAME/` tree (an APP-keyed
/// disc title) into `output_dir`'s `dev_bdvd/<title-id>/` tree.
///
/// The record's source hash is over `image` itself, so the image must
/// already be plaintext. There is no RAP -- disc EBOOTs are APP-keyed
/// -- so the decrypt-proof runs through `sce::decrypt_self_to_elf`
/// under `keys`.
///
/// # Errors
///
/// [`GameInstallError::DiscImageEncrypted`] for an image still carrying
/// its disc encryption, refused before anything is staged.
#[cfg(feature = "decrypt")]
pub fn install_iso(
    image: &[u8],
    keys: &KeyVault,
    output_dir: &Path,
    opts: InstallOptions<'_>,
) -> Result<GameInstallOutcome, GameInstallError> {
    let progress = opts.progress;
    progress.phase(Phase::Reading.code());
    let entries = iso::read_iso(image)?;

    let sfo_entry = entries
        .iter()
        .find(|e| e.path == "PS3_GAME/PARAM.SFO")
        .ok_or(GameInstallError::NoDiscParamSfo)?;
    let sfo_bytes = sfo_entry.read_data(image)?;
    let (title_id, category, title, app_version) = match parse_identity(&sfo_bytes) {
        Err(GameInstallError::Sfo(param_sfo::SfoError::BadMagic(head))) => {
            return Err(GameInstallError::DiscImageEncrypted {
                path: "PS3_GAME/PARAM.SFO",
                head,
            });
        }
        other => other?,
    };
    if !DISC_CATEGORIES.contains(&category.as_str()) {
        return Err(GameInstallError::NotDiscGame { category });
    }
    // The title-id becomes the dev_bdvd/<title-id> directory key.
    validate_content_id(&title_id)?;

    let eboot_bytes = entries
        .iter()
        .find(|e| e.path == "PS3_GAME/USRDIR/EBOOT.BIN")
        .ok_or(GameInstallError::NoDiscEboot)?
        .read_data(image)?;
    // Metadata files can sit in a plaintext region of an otherwise
    // encrypted disc, so the EBOOT settles whether the image is
    // decrypted, before the tree is streamed to disk. Disc encryption
    // is an unpadded per-sector block cipher and keeps every file's
    // length, so an EBOOT too short to hold a magic is left for the
    // proof to refuse by length. (RPCS3 `Loader/ISO.cpp` reads
    // plaintext images only.)
    if let Some(&head) = eboot_bytes.first_chunk::<4>() {
        if !is_sce_wrapped(&eboot_bytes) && head != ELF_MAGIC {
            return Err(GameInstallError::DiscImageEncrypted {
                path: "PS3_GAME/USRDIR/EBOOT.BIN",
                head,
            });
        }
    }

    let layout = StoreLayout::new(output_dir);
    let final_dir = output_dir.join("dev_bdvd").join(&title_id);
    let staging_dir = staging_sibling(&final_dir);
    // Resolved before staging, as in `install_pkg`.
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(&title_id)?,
    };
    let store_path = layout.store_path_of(&final_dir)?;

    if dir_non_empty(&final_dir)? && !opts.force {
        return Err(GameInstallError::TargetExists { path: final_dir });
    }

    // Each file stays in the image until stage_tree streams it to disk,
    // so a full BD image installs in bounded memory.
    let staged: Vec<StagedFile> = entries
        .iter()
        .map(|e| {
            Ok(StagedFile {
                path: e.path.clone(),
                is_dir: e.kind == iso::IsoEntryKind::Directory,
                data: StagedData::Slices(e.extent_slices(image)?),
            })
        })
        .collect::<Result<_, iso::IsoError>>()?;
    emit_totals(progress, &staged);

    prepare_staging(&staging_dir, progress)?;
    let file_digests = run_or_clean(&staging_dir, || {
        progress.phase(Phase::Staging.code());
        let file_digests = stage_tree(&staged, &staging_dir, progress)?;
        // APP-keyed disc EBOOT: prove it decrypts end-to-end, discard.
        progress.phase(Phase::Proving.code());
        sce::decrypt_self_to_elf(&eboot_bytes, keys).map_err(GameInstallError::DecryptProof)?;
        Ok(file_digests)
    })?;

    progress.phase(Phase::Hashing.code());
    let record = build_record(
        "iso",
        image,
        ArtifactRecord {
            kind: artifact.kind(),
            version: app_version,
            store_path,
        },
        file_digests,
        TitleRecord {
            title_id: title_id.clone(),
            content_id: title_id.clone(),
            category,
            title,
            distribution: "disc-iso".to_string(),
        },
        None,
    );
    // Disc: the staging root is itself the tree (no `tree/` nesting),
    // and there is no RAP.
    let record_path = commit(
        &staging_dir,
        &staging_dir,
        &final_dir,
        None,
        &layout.record_path(&artifact),
        &record,
        progress,
    )?;

    progress.finished();
    Ok(GameInstallOutcome {
        title_id: title_id.clone(),
        content_id: title_id,
        game_dir: final_dir,
        rap_installed: false,
        file_count: record.files.len(),
        record_path,
    })
}

/// Parse the shared identity fields from a PARAM.SFO blob, returning
/// `(title_id, category, title, app_version)`.
fn parse_identity(sfo_bytes: &[u8]) -> Result<(String, String, String, String), GameInstallError> {
    let sfo = param_sfo::parse(sfo_bytes)?;
    let title_id = sfo
        .get_string("TITLE_ID")
        .ok_or(GameInstallError::MissingTitleId)?
        .to_string();
    let category = sfo.get_string("CATEGORY").unwrap_or_default().to_string();
    let title = sfo.get_string("TITLE").unwrap_or_default().to_string();
    let app_version = sfo
        .get_string("APP_VER")
        .or_else(|| sfo.get_string("VERSION"))
        .unwrap_or_default()
        .to_string();
    Ok((title_id, category, title, app_version))
}

/// Clear and recreate a staging directory, so no foreign residue
/// survives into the commit rename.
fn prepare_staging(
    staging_dir: &Path,
    progress: &dyn ProgressSink,
) -> Result<(), GameInstallError> {
    // An interrupted multi-GB install leaves a staging tree whose
    // removal takes minutes; under the reading phase that reads as a
    // stall.
    if staging_dir.exists() {
        progress.phase(Phase::ClearingStaging.code());
    }
    match std::fs::remove_dir_all(staging_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err("remove", staging_dir)(e)),
    }
    std::fs::create_dir_all(staging_dir).map_err(io_err("create dir", staging_dir))
}

/// Run `f`, removing the staging directory if it fails.
///
/// A cleanup that cannot remove the root falsifies the module's
/// discard-whole invariant, so it surfaces as
/// [`GameInstallError::StagingResidue`] carrying the original fault
/// rather than being dropped.
fn run_or_clean<T>(
    staging_dir: &Path,
    f: impl FnOnce() -> Result<T, GameInstallError>,
) -> Result<T, GameInstallError> {
    match f() {
        Ok(v) => Ok(v),
        Err(e) => match std::fs::remove_dir_all(staging_dir) {
            Ok(()) => Err(e),
            Err(c) if c.kind() == std::io::ErrorKind::NotFound => Err(e),
            Err(c) => Err(GameInstallError::StagingResidue {
                path: staging_dir.to_path_buf(),
                source: c,
                cause: Box::new(e),
            }),
        },
    }
}

/// Write every staged entry into `tree_dest`, creating directories and
/// `fsync`-ing files, hashing each file's bytes as they are written.
///
/// Returns the per-file digests keyed by normalized path, in the same
/// order [`InstallRecord::files`] serialises them. Entries whose paths
/// normalize to one key overwrite both the file and its digest, so the
/// record hashes the bytes the tree ends up holding (last writer wins,
/// matching the on-disk overwrite).
fn stage_tree(
    staged: &[StagedFile<'_>],
    tree_dest: &Path,
    progress: &dyn ProgressSink,
) -> Result<BTreeMap<String, HexSha256>, GameInstallError> {
    // Create the destination root up front so an empty tree still has a
    // directory for the commit rename to move.
    std::fs::create_dir_all(tree_dest).map_err(io_err("create dir", tree_dest))?;
    let mut digests = BTreeMap::new();
    for f in staged {
        let dest = safe_join(tree_dest, &f.path)?;
        if f.is_dir {
            std::fs::create_dir_all(&dest).map_err(io_err("create dir", &dest))?;
        } else {
            progress.item_started(&f.path);
            let digest = match &f.data {
                StagedData::Bytes(bytes) => {
                    write_chunks_and_sync(&dest, std::iter::once(*bytes), progress)?
                }
                StagedData::Slices(slices) => {
                    write_chunks_and_sync(&dest, slices.iter().copied(), progress)?
                }
            };
            digests.insert(normalized_rel(&f.path), digest);
            progress.item_finished();
        }
    }
    Ok(digests)
}

/// Commit a proven staging batch in a fixed rename sequence: RAP into
/// `exdata/` first, then the game tree into `final_dir` (the commit
/// point), then the record last. No rollback -- see the module
/// invariants for the single residue window between the two renames.
#[allow(
    clippy::too_many_arguments,
    reason = "one private call sequence shared by two installers; a params struct would outnumber its fields with plumbing"
)]
fn commit(
    staging_root: &Path,
    tree_staging: &Path,
    final_dir: &Path,
    staged_rap: Option<&StagedRap>,
    record_path: &Path,
    record: &InstallRecord,
    progress: &dyn ProgressSink,
) -> Result<PathBuf, GameInstallError> {
    progress.phase(Phase::Committing.code());
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
    }

    // RAP first; an orphan RAP is inert (see the module invariants).
    // rename replaces an existing destination file on both platforms.
    if let Some(sr) = staged_rap {
        if let Some(parent) = sr.final_path.parent() {
            std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
        }
        std::fs::rename(&sr.staged_path, &sr.final_path).map_err(|source| {
            GameInstallError::Io {
                op: "rename",
                path: sr.final_path.clone(),
                source,
            }
        })?;
    }

    // Tree second: the commit point. Clear an existing target first
    // (the dir_non_empty gate already required --force to reach here
    // with a non-empty target). Its own phase: removing a large
    // existing install is genuinely slow.
    if final_dir.exists() {
        progress.phase(Phase::Clearing.code());
        std::fs::remove_dir_all(final_dir).map_err(io_err("remove", final_dir))?;
        progress.phase(Phase::Committing.code());
    }
    std::fs::rename(tree_staging, final_dir).map_err(|source| GameInstallError::Io {
        op: "rename",
        path: final_dir.to_path_buf(),
        source,
    })?;
    // Drop the now-treeless staging root; best-effort (already gone for
    // the disc path, where the root was itself the tree).
    std::fs::remove_dir_all(staging_root).ok();

    // Record last: a record never points at an absent tree.
    if let Some(parent) = record_path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
    }
    let text = record.to_toml()?;
    std::fs::write(record_path, text).map_err(io_err("write", record_path))?;
    Ok(record_path.to_path_buf())
}

/// Build the install record from [`stage_tree`]'s digests. File hashes
/// are over the bytes as written and keyed by path -- content-only, no
/// mtimes or permissions.
fn build_record(
    kind: &str,
    source_bytes: &[u8],
    artifact: ArtifactRecord,
    files: BTreeMap<String, HexSha256>,
    title: TitleRecord,
    rap: Option<RapRecord>,
) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact,
        source: SourceRecord::local(kind, sha256_of(source_bytes)),
        title: Some(title),
        files,
        rap,
    }
}

#[cfg(test)]
#[path = "tests/game_install_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/game_install_disc_tests.rs"]
mod disc_tests;
