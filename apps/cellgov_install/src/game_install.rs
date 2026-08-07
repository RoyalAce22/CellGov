//! Game-install orchestration: turn a retail PKG or disc image into a
//! `dev_hdd0`/`dev_bdvd` tree, an installed RAP, and an
//! `installs/<title-id>.install.toml` record.
//!
//! The pure container work lives in [`crate::pkg`] / [`crate::iso`];
//! this module owns the filesystem shell.
//!
//! # Invariants
//!
//! - The whole pre-commit batch -- game tree, staged RAP, and the
//!   decrypt-proof -- lives under one staging root
//!   (`.staging-<title-id>/`). A fault anywhere before commit discards
//!   that root whole, so a failed install leaves zero `exdata` residue.
//! - Commit runs only after the decrypt-proof passes, as a short fixed
//!   rename sequence: the staged RAP into `exdata/` first, then the
//!   game tree into its final directory (the commit point), then the
//!   record last. The single residue window is between those two
//!   renames -- a tree-rename failure leaves an inert, content-id-keyed
//!   RAP with no game directory, read only when that title's EBOOT is
//!   decrypted and overwritten identically on retry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::iso;
use crate::manifest::Sha256 as HexSha256;
use crate::npdrm::{self, NpdHeaderInfo, NpdLicense};
use crate::param_sfo;
use crate::pkg::{self, PkgEntryKind};
use crate::sce;

/// Install-record schema version; bumped on any breaking layout change.
/// v2 added the optional `[rap]` table.
pub const INSTALL_RECORD_FORMAT_VERSION: u32 = 2;

/// The single modeled user profile, matching the boot path's
/// `home/00000001/exdata` RAP lookup. Shared with [`crate::game_uninstall`].
pub(crate) const HDD0_USER: &str = "00000001";

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
    /// The record entry (filename + hash) for the install record.
    record: RapRecord,
}

/// One file or directory queued for staging, normalized across the PKG
/// and ISO inputs so the commit machinery is container-agnostic.
struct StagedFile {
    /// Target-relative path, `/`-separated.
    path: String,
    /// Whether this is a directory (no bytes).
    is_dir: bool,
    /// File bytes (empty for a directory).
    data: Vec<u8>,
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
    /// Number of files written to the game tree.
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
}

/// Source-container provenance for an install record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    /// Container kind: `pkg` or `iso`.
    pub kind: String,
    /// SHA-256 over the source container bytes.
    pub sha256: HexSha256,
}

/// The RAP installed for an NPDRM title, recorded so uninstall can
/// locate and verify it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RapRecord {
    /// RAP filename under `home/00000001/exdata/` (the full NPD
    /// content-id plus `.rap`).
    pub filename: String,
    /// SHA-256 over the installed RAP bytes.
    pub sha256: HexSha256,
}

/// Title identity for an install record, all PARAM.SFO-derived.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleRecord {
    /// PARAM.SFO `TITLE_ID` (game-directory key).
    pub title_id: String,
    /// Full NPD content id (RAP-filename key) or the title-id.
    pub content_id: String,
    /// PARAM.SFO `CATEGORY`.
    pub category: String,
    /// PARAM.SFO `TITLE`.
    pub title: String,
    /// PARAM.SFO `APP_VER` (falling back to `VERSION`).
    pub app_version: String,
    /// Install distribution tag (`psn-hdd` / `disc-iso`).
    pub distribution: String,
}

/// The `installs/<title-id>.install.toml` record: enough to verify a
/// reinstall reproduces the same tree from the same source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRecord {
    /// Schema version.
    pub format_version: u32,
    /// Source container provenance.
    pub source: SourceRecord,
    /// Title identity.
    pub title: TitleRecord,
    /// Per-file SHA-256, keyed by game-tree-relative path. A
    /// `BTreeMap` serialises as a single `[files]` table sorted by
    /// path, so the byte output is a pure function of the tree.
    pub files: BTreeMap<String, HexSha256>,
    /// The installed RAP, when the title is NPDRM with a network/local
    /// license. Absent (and omitted from the TOML) for disc, free, and
    /// no-RAP titles.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rap: Option<RapRecord>,
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

/// Write `bytes` to `path`, creating parents, and `fsync` the file so
/// the content is durable before the commit rename.
fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<(), GameInstallError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
    }
    let f = std::fs::File::create(path).map_err(io_err("create", path))?;
    {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(&f);
        w.write_all(bytes).map_err(io_err("write", path))?;
        w.flush().map_err(io_err("flush", path))?;
    }
    f.sync_all().map_err(io_err("fsync", path))?;
    Ok(())
}

/// Join a container-relative entry path under `base`, rejecting any
/// component that could escape it. Only `Normal` components extend the
/// path; `.` is dropped; an absolute root, drive prefix, or `..` is a
/// hard error.
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
    Ok(out)
}

/// The staging-relative path a staged entry lands at, as a
/// `/`-separated string built from the same `Normal` components
/// [`safe_join`] keeps. Drops `.` and collapses odd separators so the
/// record key equals the on-disk path -- keeping the record a pure
/// function of the installed tree, not the container's path spelling.
/// Identity for the clean `/`-separated paths real PKGs/ISOs carry.
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

/// Reject a content-id that is empty or carries a byte outside
/// `[A-Za-z0-9._-]`, before it is used to build a game-directory or
/// RAP-file path.
fn validate_content_id(id: &str) -> Result<(), GameInstallError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(GameInstallError::UnsafeContentId {
            content_id: id.to_string(),
        });
    }
    Ok(())
}

/// Whether a license consumes a RAP. Only network/local do; free and
/// APP-keyed (no NPD header) titles resolve their klicensee without one
/// (free falls back to `NP_KLIC_FREE`). Match every `NpdLicense` variant
/// explicitly: a `matches!(.. A | B)` would silently treat any new
/// variant as "no RAP", so a future addition must make a conscious
/// choice here.
fn rap_consumed(license: Option<NpdLicense>) -> bool {
    match license {
        Some(NpdLicense::Network) | Some(NpdLicense::Local) => true,
        Some(NpdLicense::Free) | None => false,
    }
}

/// The RAP to stage for a title, or `None` for disc/free/no-RAP titles.
///
/// The whole network-vs-free decision lives here, isolated from the
/// filesystem so the gate is testable without a passing decrypt-proof:
/// `rap_needed` is true only for a network/local license, so a RAP
/// supplied for a free title is dropped here and never staged, committed,
/// or recorded.
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
/// `dev_hdd0` tree, installing the RAP and writing a record into
/// `installs_dir`.
pub fn install_pkg(
    pkg_bytes: &[u8],
    rap: Option<&[u8]>,
    output_dir: &Path,
    installs_dir: &Path,
    force: bool,
) -> Result<GameInstallOutcome, GameInstallError> {
    let archive = pkg::extract(pkg_bytes)?;

    // PARAM.SFO -> identity + HDD-game gate.
    let sfo_file = archive
        .files
        .iter()
        .find(|f| f.name == "PARAM.SFO")
        .ok_or(GameInstallError::NoParamSfo)?;
    let (title_id, category, title, app_version) = parse_identity(&sfo_file.data)?;
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
    let npd = npdrm::find_npd_header_info(&eboot.data).map_err(GameInstallError::Npd)?;
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
    let dev_hdd0 = output_dir.join("dev_hdd0");
    let exdata = dev_hdd0.join("home").join(HDD0_USER).join("exdata");
    let game_root = dev_hdd0.join("game");
    let final_dir = game_root.join(&title_id);
    let staging_root = game_root.join(format!(".staging-{title_id}"));
    let tree_staging = staging_root.join("tree");

    if dir_non_empty(&final_dir)? && !force {
        return Err(GameInstallError::TargetExists { path: final_dir });
    }

    // Fail fast on the RAP contract before staging anything. Only
    // network/local licenses consume a RAP; a RAP supplied for a free
    // title is ignored.
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
            data: f.data.clone(),
        })
        .collect();

    // Build the whole batch (tree + RAP + proof) under the staging
    // root; any failure discards it whole, leaving no exdata residue.
    prepare_staging(&staging_root)?;
    let staged_rap = run_or_clean(&staging_root, || {
        stage_tree(&staged, &tree_staging)?;
        // Stage the RAP under the root (network/local only).
        // `plan_staged_rap` owns the network-vs-free decision; a RAP for
        // a free title is dropped there, and a free title with no RAP
        // uses the NP_KLIC_FREE fallback in the resolver below.
        let staged_rap = plan_staged_rap(rap_needed, rap, &content_id, &staging_root, &exdata);
        if let Some(sr) = &staged_rap {
            let rap_bytes = rap.expect(
                "invariant: plan_staged_rap returns Some only when rap_needed, so rap is set",
            );
            write_and_sync(&sr.staged_path, rap_bytes)?;
        }
        // The proof resolves the RAP the way the load path will -- a
        // content-id-keyed read followed by `rap_to_klic` -- but
        // against the staged copy, so a fault before commit touches no
        // live exdata.
        let rap_staging = staging_root.join("rap");
        let resolver = move |n: &NpdHeaderInfo| -> Option<[u8; 16]> {
            let rap_path = rap_staging.join(format!("{}.rap", n.content_id));
            let bytes = std::fs::read(&rap_path).ok()?;
            let arr: [u8; 16] = bytes.as_slice().try_into().ok()?;
            Some(npdrm::rap_to_klic(&arr))
        };
        npdrm::decrypt_self_to_elf_auto(&eboot.data, resolver)
            .map_err(GameInstallError::DecryptProof)?;
        Ok(staged_rap)
    })?;

    let rap_installed = staged_rap.is_some();
    let record = build_record(
        "pkg",
        pkg_bytes,
        &staged,
        TitleRecord {
            title_id: title_id.clone(),
            content_id: content_id.clone(),
            category,
            title,
            app_version,
            distribution: "psn-hdd".to_string(),
        },
        staged_rap.as_ref().map(|r| r.record.clone()),
    );
    let record_path = commit(
        &staging_root,
        &tree_staging,
        &final_dir,
        staged_rap.as_ref(),
        installs_dir,
        &title_id,
        &record,
    )?;

    Ok(GameInstallOutcome {
        title_id,
        content_id,
        game_dir: final_dir,
        rap_installed,
        file_count: count_files(&staged),
        record_path,
    })
}

/// Install a decrypted disc image's `PS3_GAME/` tree (an APP-keyed
/// disc title) into `output_dir`'s `dev_bdvd/<title-id>/` tree.
///
/// The image is already decrypted (the encrypted-disc key pass runs
/// upstream). There is no RAP -- disc EBOOTs are APP-keyed -- so the
/// decrypt-proof runs through [`sce::decrypt_self_to_elf`].
pub fn install_iso(
    image: &[u8],
    source_bytes: &[u8],
    output_dir: &Path,
    installs_dir: &Path,
    force: bool,
) -> Result<GameInstallOutcome, GameInstallError> {
    let entries = iso::read_iso(image)?;

    let sfo_entry = entries
        .iter()
        .find(|e| e.path == "PS3_GAME/PARAM.SFO")
        .ok_or(GameInstallError::NoDiscParamSfo)?;
    let (title_id, category, title, app_version) = parse_identity(&sfo_entry.data)?;
    if !DISC_CATEGORIES.contains(&category.as_str()) {
        return Err(GameInstallError::NotDiscGame { category });
    }
    // The title-id becomes the dev_bdvd/<title-id> directory key.
    validate_content_id(&title_id)?;

    let eboot = entries
        .iter()
        .find(|e| e.path == "PS3_GAME/USRDIR/EBOOT.BIN")
        .ok_or(GameInstallError::NoDiscEboot)?;

    let dev_bdvd = output_dir.join("dev_bdvd");
    let final_dir = dev_bdvd.join(&title_id);
    let staging_dir = dev_bdvd.join(format!(".staging-{title_id}"));

    if dir_non_empty(&final_dir)? && !force {
        return Err(GameInstallError::TargetExists { path: final_dir });
    }

    let staged: Vec<StagedFile> = entries
        .iter()
        .map(|e| StagedFile {
            path: e.path.clone(),
            is_dir: e.kind == iso::IsoEntryKind::Directory,
            data: e.data.clone(),
        })
        .collect();

    prepare_staging(&staging_dir)?;
    run_or_clean(&staging_dir, || {
        stage_tree(&staged, &staging_dir)?;
        // APP-keyed disc EBOOT: prove it decrypts end-to-end, discard.
        sce::decrypt_self_to_elf(&eboot.data).map_err(GameInstallError::DecryptProof)?;
        Ok(())
    })?;

    let record = build_record(
        "iso",
        source_bytes,
        &staged,
        TitleRecord {
            title_id: title_id.clone(),
            content_id: title_id.clone(),
            category,
            title,
            app_version,
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
        installs_dir,
        &title_id,
        &record,
    )?;

    Ok(GameInstallOutcome {
        title_id: title_id.clone(),
        content_id: title_id,
        game_dir: final_dir,
        rap_installed: false,
        file_count: count_files(&staged),
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

/// Number of file (non-directory) entries in a staged tree.
fn count_files(staged: &[StagedFile]) -> usize {
    staged.iter().filter(|f| !f.is_dir).count()
}

/// Clear and recreate a staging directory. A pre-existing staging dir
/// is removed; its absence is fine, but any other removal error
/// surfaces (a swallowed error could leave foreign residue that the
/// commit rename would then carry into the live tree).
fn prepare_staging(staging_dir: &Path) -> Result<(), GameInstallError> {
    match std::fs::remove_dir_all(staging_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err("remove", staging_dir)(e)),
    }
    std::fs::create_dir_all(staging_dir).map_err(io_err("create dir", staging_dir))
}

/// Run `f`, removing the staging directory if it fails. Used to keep a
/// partial tree from outliving a pre-commit error.
fn run_or_clean<T>(
    staging_dir: &Path,
    f: impl FnOnce() -> Result<T, GameInstallError>,
) -> Result<T, GameInstallError> {
    match f() {
        Ok(v) => Ok(v),
        Err(e) => {
            std::fs::remove_dir_all(staging_dir).ok();
            Err(e)
        }
    }
}

/// Write every staged entry into `staging_dir`, creating directories
/// and `fsync`-ing files.
fn stage_tree(staged: &[StagedFile], tree_dest: &Path) -> Result<(), GameInstallError> {
    // Create the destination root up front so an empty tree still has a
    // directory for the commit rename to move.
    std::fs::create_dir_all(tree_dest).map_err(io_err("create dir", tree_dest))?;
    for f in staged {
        let dest = safe_join(tree_dest, &f.path)?;
        if f.is_dir {
            std::fs::create_dir_all(&dest).map_err(io_err("create dir", &dest))?;
        } else {
            write_and_sync(&dest, &f.data)?;
        }
    }
    Ok(())
}

/// Commit a proven staging batch in a fixed rename sequence: RAP into
/// `exdata/` first, then the game tree into `final_dir` (the commit
/// point), then the record last. No rollback -- see the module
/// invariants for the single residue window between the two renames.
fn commit(
    staging_root: &Path,
    tree_staging: &Path,
    final_dir: &Path,
    staged_rap: Option<&StagedRap>,
    installs_dir: &Path,
    title_id: &str,
    record: &InstallRecord,
) -> Result<PathBuf, GameInstallError> {
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent).map_err(io_err("create dir", parent))?;
    }

    // RAP first. An orphan RAP (if the tree rename then fails) is
    // inert: it is read only when its content-id's game tree is
    // decrypted, and a retry overwrites it with identical bytes. rename
    // replaces an existing destination file on both platforms.
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
    // with a non-empty target).
    if final_dir.exists() {
        std::fs::remove_dir_all(final_dir).map_err(io_err("remove", final_dir))?;
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
    let record_path = installs_dir.join(format!("{title_id}.install.toml"));
    std::fs::create_dir_all(installs_dir).map_err(io_err("create dir", installs_dir))?;
    let text = toml::to_string(record)?;
    std::fs::write(&record_path, text).map_err(io_err("write", &record_path))?;
    Ok(record_path)
}

/// Build the install record from a staged tree. File hashes are over
/// the bytes as written and keyed by path -- content-only, no mtimes
/// or permissions. The `BTreeMap` keeps the output sorted by path.
fn build_record(
    kind: &str,
    source_bytes: &[u8],
    staged: &[StagedFile],
    title: TitleRecord,
    rap: Option<RapRecord>,
) -> InstallRecord {
    let files: BTreeMap<String, HexSha256> = staged
        .iter()
        .filter(|f| !f.is_dir)
        .map(|f| (normalized_rel(&f.path), sha256_of(&f.data)))
        .collect();

    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        source: SourceRecord {
            kind: kind.to_string(),
            sha256: sha256_of(source_bytes),
        },
        title,
        files,
        rap,
    }
}

#[cfg(test)]
#[path = "tests/game_install_tests.rs"]
mod tests;
