//! Mount-table resolution with single-read disk caching.

use std::collections::BTreeMap;
use std::fs::Metadata;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use cellgov_ps3_abi::lv2::errno;

use crate::fs_store::{DirEntry, FsError};
use crate::host::Lv2Host;

/// Outcome of a host-side mount-table lookup for a regular-file path.
pub(super) enum MountResolution {
    /// No mount prefix matched.
    Unmounted,
    /// Bytes were read from the host file and registered as a blob
    /// under the original guest path; caller should re-query the
    /// in-memory FS.
    Cached,
    /// Mount matched but the host-side lookup failed.
    Failed(cellgov_ps3_abi::lv2::errno::Lv2ErrCode),
}

/// Outcome of a host-side mount-table lookup for a directory path.
///
/// Each `sys_fs_opendir` snapshots the host directory fresh; nothing
/// is pre-cached in `FsStore`.
pub(super) enum DirMountResolution {
    Unmounted,
    /// Entries sorted lexicographically; symlinks, special files, and
    /// non-UTF-8 names dropped.
    Snapshot(Vec<DirEntry>),
    Failed(cellgov_ps3_abi::lv2::errno::Lv2ErrCode),
}

impl Lv2Host {
    /// Try to satisfy a guest path via the mount table, caching the
    /// host file's bytes as a blob keyed on the guest path.
    ///
    /// Determinism contract: a single host read per guest path; the
    /// cached content is immutable thereafter.
    pub(super) fn try_mount_resolve_and_cache(&mut self, path: &str) -> MountResolution {
        let candidates = match resolve_candidates(self, path) {
            Ok(c) => c,
            Err(MountResolveErr::Unmounted) => return MountResolution::Unmounted,
            Err(MountResolveErr::Failed(code)) => return MountResolution::Failed(code),
        };

        let host_path = match first_existing(&candidates) {
            Ok(Some((host_path, md))) if md.is_file() => host_path.to_path_buf(),
            // A shadowing root that holds a directory under this name
            // hides whatever a later root holds there.
            Ok(Some(_)) | Ok(None) => return MountResolution::Failed(errno::CELL_ENOENT),
            Err((candidate, kind)) => {
                let code = self.mount_candidate_unreadable(path, candidate, kind);
                return MountResolution::Failed(code);
            }
        };

        let bytes = match std::fs::read(&host_path) {
            Ok(b) => b,
            Err(_) => return MountResolution::Failed(errno::CELL_EIO),
        };

        match self.fs_store_mut().register_blob(path.to_string(), bytes) {
            Ok(()) => MountResolution::Cached,
            Err(FsError::PathAlreadyRegistered) => {
                self.record_invariant_break(
                    "dispatch.fs.mount_register_double",
                    format_args!(
                        "register_blob returned PathAlreadyRegistered for {path:?} \
                         after UnknownPath; contract violated"
                    ),
                );
                MountResolution::Failed(errno::CELL_EFAULT)
            }
            Err(other) => {
                self.record_invariant_break(
                    "dispatch.fs.mount_register_unexpected",
                    format_args!(
                        "register_blob returned {other:?} for {path:?}; contract violated"
                    ),
                );
                MountResolution::Failed(errno::CELL_EFAULT)
            }
        }
    }

    /// Try to satisfy a guest directory path via the mount table,
    /// merged across every root that holds the directory.
    ///
    /// Determinism contract:
    /// - Entries sorted by `name` in lexicographic byte order.
    /// - The earliest root that holds a name supplies its entry;
    ///   later roots do not change its type.
    /// - Symlinks, special files, and non-UTF-8 names are dropped.
    /// - A root the host will not describe fails the whole listing.
    pub(super) fn try_mount_resolve_dir(&mut self, path: &str) -> DirMountResolution {
        let candidates = match resolve_candidates(self, path) {
            Ok(c) => c,
            Err(MountResolveErr::Unmounted) => return DirMountResolution::Unmounted,
            Err(MountResolveErr::Failed(code)) => return DirMountResolution::Failed(code),
        };

        // One probe pass over the roots. The same metadata decides
        // the type of the hit and which roots contribute entries, so
        // the listing cannot straddle two disk states.
        let mut present: Vec<(&Path, Metadata)> = Vec::new();
        for candidate in &candidates {
            match probe(candidate) {
                Ok(md) => {
                    // The earliest root that holds this name decides
                    // the type; a file there hides every later
                    // root's directory.
                    if present.is_empty() && !md.is_dir() {
                        return DirMountResolution::Failed(errno::CELL_ENOTDIR);
                    }
                    present.push((candidate.as_path(), md));
                }
                Err(CandidateMiss::Absent) => {}
                Err(CandidateMiss::Unreadable(kind)) => {
                    let code = self.mount_candidate_unreadable(path, candidate, kind);
                    return DirMountResolution::Failed(code);
                }
            }
        }
        if present.is_empty() {
            return DirMountResolution::Failed(errno::CELL_ENOENT);
        }

        // The `String` key gives the UTF-8 byte order the contract
        // names, and `or_insert` keeps the earliest root's entry.
        let mut merged: BTreeMap<String, DirEntry> = BTreeMap::new();
        for (candidate, md) in &present {
            if !md.is_dir() {
                continue;
            }
            if let Err(kind) = collect_dir_entries(candidate, &mut merged) {
                let code = self.mount_candidate_unreadable(path, candidate, kind);
                return DirMountResolution::Failed(code);
            }
        }
        DirMountResolution::Snapshot(merged.into_values().collect())
    }

    /// Log a candidate root the host would not read, and map its
    /// error kind to the errno the guest sees.
    fn mount_candidate_unreadable(
        &mut self,
        path: &str,
        candidate: &Path,
        kind: ErrorKind,
    ) -> cellgov_ps3_abi::lv2::errno::Lv2ErrCode {
        self.log_invariant_break(
            "dispatch.fs.mount_candidate_unreadable",
            format_args!(
                "host lookup of {candidate:?} while resolving {path:?} failed with \
                 {kind:?}; refusing to fall through to a later root"
            ),
        );
        if kind == ErrorKind::PermissionDenied {
            errno::CELL_EACCES
        } else {
            errno::CELL_EIO
        }
    }
}

/// Read one host directory into `merged`; a repeated name keeps the
/// entry already there.
///
/// # Errors
///
/// Returns the host error kind if the host will not enumerate the
/// directory. The caller maps that kind to a guest errno.
fn collect_dir_entries(
    host_path: &Path,
    merged: &mut BTreeMap<String, DirEntry>,
) -> Result<(), ErrorKind> {
    let read_dir = std::fs::read_dir(host_path).map_err(|e| e.kind())?;
    for entry in read_dir {
        let entry = entry.map_err(|e| e.kind())?;
        let file_type = entry.file_type().map_err(|e| e.kind())?;
        let is_directory = if file_type.is_dir() {
            true
        } else if file_type.is_file() {
            false
        } else {
            continue;
        };
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        merged
            .entry(name.clone())
            .or_insert(DirEntry { name, is_directory });
    }
    Ok(())
}

/// Why a probed root does not answer for a guest path.
enum CandidateMiss {
    /// The name is not under this root.
    Absent,
    /// The host would not say what this root holds.
    Unreadable(ErrorKind),
}

/// Probe one candidate and tell an absent name apart from an
/// unreadable root.
///
/// Three host error kinds report one fact -- nothing is under this
/// root at that name:
/// - `NotFound`.
/// - `NotADirectory`, from a host family where a path component is
///   a regular file.
/// - `InvalidFilename`, from a host that cannot express the name.
///
/// Every other kind leaves the root's contents unknown.
fn probe(candidate: &Path) -> Result<Metadata, CandidateMiss> {
    match std::fs::metadata(candidate) {
        Ok(md) => Ok(md),
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::InvalidFilename
            ) =>
        {
            Err(CandidateMiss::Absent)
        }
        Err(err) => Err(CandidateMiss::Unreadable(err.kind())),
    }
}

/// First candidate that exists on the host, with its metadata.
///
/// # Errors
///
/// Returns the first unreadable candidate and its host error kind.
fn first_existing(candidates: &[PathBuf]) -> Result<Option<(&Path, Metadata)>, (&Path, ErrorKind)> {
    for candidate in candidates {
        match probe(candidate) {
            Ok(md) => return Ok(Some((candidate.as_path(), md))),
            Err(CandidateMiss::Absent) => {}
            Err(CandidateMiss::Unreadable(kind)) => return Err((candidate.as_path(), kind)),
        }
    }
    Ok(None)
}

#[derive(Debug, thiserror::Error)]
enum MountResolveErr {
    #[error("no mount matched")]
    Unmounted,
    #[error("mount resolve failed: lv2 errno 0x{:08x}", .0.code)]
    Failed(cellgov_ps3_abi::lv2::errno::Lv2ErrCode),
}

/// Shared prefix-resolution step for the file and directory surfaces.
fn resolve_candidates(host: &mut Lv2Host, path: &str) -> Result<Vec<PathBuf>, MountResolveErr> {
    match host.fs_mounts().resolve_candidates(path) {
        Ok(Some(c)) => Ok(c),
        Ok(None) => Err(MountResolveErr::Unmounted),
        Err(FsError::PathTraversal) => Err(MountResolveErr::Failed(errno::CELL_EACCES)),
        Err(other) => {
            host.record_invariant_break(
                "dispatch.fs.mount_resolve_unexpected",
                format_args!(
                    "FsMountTable::resolve_candidates returned {other:?} for {path:?}; \
                     contract violated"
                ),
            );
            Err(MountResolveErr::Failed(errno::CELL_EFAULT))
        }
    }
}

#[cfg(test)]
#[path = "tests/overlay_tests.rs"]
mod tests;
