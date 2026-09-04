//! The filesystem shell every installer shares: stage a whole batch
//! under one hidden sibling of the target, then commit it with renames.
//!
//! # Invariants
//!
//! - The whole pre-commit batch -- the tree, and whichever of a staged
//!   RAP and a decrypt-proof the installer has -- lives under one
//!   [`staging_sibling`](crate::store::staging_sibling) of the target
//!   directory; a fault before commit discards it whole.
//! - [`commit`] is a fixed sequence:
//!   1. rename the staged RAP into `exdata/`,
//!   2. remove the record the target already has,
//!   3. rename the staged tree onto the target -- the commit point,
//!   4. write the new record.
//!
//!   A record therefore never names an absent tree. Nothing syncs a
//!   directory, so a host crash can still reorder the unlink and the
//!   rename.
//! - The residue window runs from the RAP rename to the record write.
//!   A fault that clears the target, or that writes the record, leaves
//!   a tree no record names. The target gate calls that `TargetExists`,
//!   and a `--force` retry commits over it. A fault that renames the
//!   tree leaves no target at all: the staged tree stays under the
//!   staging root for the next [`prepare_staging`], so that retry needs
//!   no `--force`. The RAP is in `exdata/` in both cases. It is inert,
//!   read only when that title's EBOOT is decrypted, and a retry
//!   overwrites it identically.
//! - The staging root's own removal, after the tree rename, is
//!   best-effort and its failure is not reported: the install has
//!   already committed, and a `.staging-<target>` left behind is swept
//!   by name by the next [`prepare_staging`] on that target.

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

use crate::game_install::error::GameInstallError;
use crate::manifest::Sha256 as HexSha256;
use crate::param_sfo;
use crate::progress::{Phase, ProgressSink};
use crate::store::layout::is_safe_component;
use crate::store::record::{
    tree_rel_path_is_safe, ArtifactRecord, InstallRecord, RapRecord, SourceRecord, TitleRecord,
    INSTALL_RECORD_FORMAT_VERSION,
};

/// Knobs shared by every installer.
#[derive(Clone, Copy)]
pub struct InstallOptions<'a> {
    /// Overwrite an occupied target: a non-empty target directory, and
    /// for the update installer an already-installed version.
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

/// A RAP staged under the staging root, ready to be renamed into
/// `exdata/` at commit and recorded in the install record.
pub(super) struct StagedRap {
    /// `staging_root/rap/<content-id>.rap`.
    pub(super) staged_path: PathBuf,
    /// `exdata/<content-id>.rap` -- the commit destination.
    pub(super) final_path: PathBuf,
    pub(super) record: RapRecord,
}

/// One file or directory queued for staging, normalized across the PKG
/// and ISO inputs so the commit machinery is container-agnostic.
pub(super) struct StagedFile<'a> {
    /// Target-relative path, `/`-separated.
    pub(super) path: String,
    pub(super) is_dir: bool,
    pub(super) data: StagedData<'a>,
}

/// Where a staged entry's bytes come from. [`stage_tree`] streams them
/// to disk one entry at a time, so neither variant requires the whole
/// tree resident -- a BD-DL disc's content exceeds host memory.
pub(super) enum StagedData<'a> {
    /// One borrowed buffer (PKG entries; empty for a directory).
    Bytes(&'a [u8]),
    /// Ordered extent slices into the source disc image (ISO entries),
    /// bounds-checked at carve time.
    Slices(Vec<&'a [u8]>),
}

pub(crate) fn sha256_of(bytes: &[u8]) -> HexSha256 {
    let mut h = Sha256::new();
    h.update(bytes);
    HexSha256(h.finalize().into())
}

pub(super) fn io_err<'a>(
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
pub(super) fn write_and_sync(path: &Path, bytes: &[u8]) -> Result<(), GameInstallError> {
    write_chunks_and_sync(path, std::iter::once(bytes), &()).map(|_| ())
}

/// Report the staging denominator: non-directory entry count and their
/// total bytes. Staged entries whose paths collide still each count,
/// so the totals can slightly exceed what lands; a renderer treats
/// them as a ceiling.
pub(super) fn emit_totals(progress: &dyn ProgressSink, staged: &[StagedFile<'_>]) {
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

/// Whether a container-relative entry path stays inside the tree it is
/// staged into: no rooting or drive prefix, no `..`, and a normalized
/// form the record gate accepts as a key.
fn entry_path_is_safe(rel: &str) -> bool {
    use std::path::Component::{CurDir, Normal, ParentDir, Prefix, RootDir};
    for comp in Path::new(rel).components() {
        match comp {
            Normal(_) | CurDir => {}
            RootDir | Prefix(_) | ParentDir => return false,
        }
    }
    // The same entry becomes a record key, and the record gate refuses
    // a key it could not resolve back onto the tree.
    tree_rel_path_is_safe(&normalized_rel(rel))
}

/// The path a container entry stages at under `prefix`, as a
/// `/`-separated string.
///
/// The raw entry is gated before the prefix goes on.
///
/// # Errors
///
/// [`GameInstallError::UnsafeEntryPath`] for an entry
/// [`entry_path_is_safe`] refuses.
pub(super) fn prefixed_entry_path(prefix: &str, rel: &str) -> Result<String, GameInstallError> {
    if !entry_path_is_safe(rel) {
        return Err(GameInstallError::UnsafeEntryPath {
            path: rel.to_string(),
        });
    }
    Ok(format!("{prefix}/{}", normalized_rel(rel)))
}

/// Join a container-relative entry path under `base`, rejecting any
/// component that could escape it.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, GameInstallError> {
    if !entry_path_is_safe(rel) {
        return Err(GameInstallError::UnsafeEntryPath {
            path: rel.to_string(),
        });
    }
    let mut out = base.to_path_buf();
    for comp in Path::new(rel).components() {
        if let std::path::Component::Normal(c) = comp {
            out.push(c);
        }
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
pub(super) fn validate_content_id(id: &str) -> Result<(), GameInstallError> {
    if is_safe_component(id) {
        Ok(())
    } else {
        Err(GameInstallError::UnsafeContentId {
            content_id: id.to_string(),
        })
    }
}

/// Whether `path` exists and contains at least one entry.
///
/// A stat that fails is not an absent directory: it refuses here
/// rather than reporting an occupied target as free.
pub(super) fn dir_non_empty(path: &Path) -> Result<bool, GameInstallError> {
    if !std::fs::exists(path).map_err(io_err("stat", path))? {
        return Ok(false);
    }
    let mut entries = std::fs::read_dir(path).map_err(io_err("read dir", path))?;
    Ok(entries.next().is_some())
}

/// Parse the shared identity fields from a PARAM.SFO blob, returning
/// `(title_id, category, title, app_version)`.
pub(super) fn parse_identity(
    sfo_bytes: &[u8],
) -> Result<(String, String, String, String), GameInstallError> {
    let sfo = param_sfo::parse(sfo_bytes)?;
    let title_id = sfo
        .get_string("TITLE_ID")
        .ok_or(GameInstallError::MissingTitleId)?
        .to_string();
    let category = sfo.get_string("CATEGORY").unwrap_or_default().to_string();
    let title = sfo.get_string("TITLE").unwrap_or_default().to_string();
    // An empty APP_VER falls through to VERSION rather than winning the
    // `or_else`: a container that carries the key with no value names no
    // version, and for an update that string is a directory name.
    let app_version = sfo
        .get_string("APP_VER")
        .filter(|v| !v.is_empty())
        .or_else(|| sfo.get_string("VERSION"))
        .unwrap_or_default()
        .to_string();
    Ok((title_id, category, title, app_version))
}

/// Clear and recreate a staging directory, so no foreign residue
/// survives into the commit rename.
pub(super) fn prepare_staging(
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
pub(super) fn run_or_clean<T>(
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
pub(super) fn stage_tree(
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

/// An absent record is success; every other failure is named.
fn remove_record(record_path: &Path) -> Result<(), GameInstallError> {
    match std::fs::remove_file(record_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("remove", record_path)(e)),
    }
}

/// Commit a staging batch in a fixed sequence: RAP into `exdata/`
/// first, then the record removed, then the tree into `final_dir` (the
/// commit point), then the record written. No rollback -- see the
/// module invariants for the residue window.
///
/// `tree_staging` may be `staging_root` itself: an installer that
/// stages no sibling `rap/` passes the root as the tree.
#[allow(
    clippy::too_many_arguments,
    reason = "one private call sequence shared by the installers; a params struct would outnumber its fields with plumbing"
)]
pub(super) fn commit(
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

    // The record goes before the tree it names, so nothing between here
    // and the write below can leave a record over an absent tree.
    remove_record(record_path)?;

    // Tree second: the commit point. Clear an existing target first
    // (the dir_non_empty gate already required --force to reach here
    // with a non-empty target). Its own phase: removing a large
    // existing install is genuinely slow.
    if std::fs::exists(final_dir).map_err(io_err("stat", final_dir))? {
        progress.phase(Phase::Clearing.code());
        std::fs::remove_dir_all(final_dir).map_err(io_err("remove", final_dir))?;
        progress.phase(Phase::Committing.code());
    }
    std::fs::rename(tree_staging, final_dir).map_err(|source| GameInstallError::Io {
        op: "rename",
        path: final_dir.to_path_buf(),
        source,
    })?;
    // Drop the now-treeless staging root; best-effort (already gone
    // wherever the root was itself the tree).
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
pub(super) fn build_record(
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
#[path = "tests/staging_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/commit_discipline_tests.rs"]
mod commit_discipline_tests;
