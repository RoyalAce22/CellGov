use cellgov_mem::lanes::{self, object_digest, source, LaneMap, LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv2::fs::LV2_FS_OBJECT_ID_BASE;
use num_enum::TryFromPrimitive;

use super::FsError;

/// Whence values for [`FsStore::seek`]. Matches PS3 `CELL_FS_SEEK_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[repr(u32)]
pub enum SeekWhence {
    /// From the start of the file.
    Set = 0,
    /// From the current fd position.
    Cur = 1,
    /// From the end of the file.
    End = 2,
}

impl SeekWhence {
    /// Returns `None` for any out-of-range value so the caller can
    /// map it to CELL_EINVAL.
    pub fn from_guest(value: u32) -> Option<Self> {
        Self::try_from(value).ok()
    }
}

/// File-stat shape returned by [`FsStore::stat_path`] / [`FsStore::fstat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    /// Blob size in bytes.
    pub size: u64,
}

/// One entry in a directory snapshot.
///
/// `read_dir_entry` walks entries in registration order; sorting
/// is the dispatcher's concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Filename (no path components). Must not contain `/` or
    /// embedded NUL; the dispatcher filters those before forwarding.
    pub name: String,
    /// `true` for a sub-directory, `false` for a regular file.
    pub is_directory: bool,
}

#[derive(Debug, Clone)]
struct BlobEntry {
    bytes: Vec<u8>,
    /// Digest of `bytes`, computed once at registration so a lane of
    /// the blob does not re-read the whole blob.
    content_digest: u64,
}

impl LaneValue for BlobEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.bytes.len() as u64);
        lanes.lane(2, 0, self.content_digest);
    }
}

#[derive(Debug, Clone)]
struct FdEntry {
    path: String,
    offset: u64,
}

impl LaneValue for FdEntry {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.bytes(1, &[], self.path.as_bytes());
        lanes.lane(2, 0, self.offset);
    }
}

#[derive(Debug, Clone)]
struct DirSnapshot {
    /// Frozen at `open_dir` time; never re-read from disk.
    entries: Vec<DirEntry>,
    /// Equal to `entries.len()` at EOF.
    cursor: usize,
}

impl LaneValue for DirSnapshot {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.cursor as u64);
        lanes.lane(2, 0, self.entries.len() as u64);
        for (slot, entry) in self.entries.iter().enumerate() {
            lanes.bytes(3, &[slot as u64], entry.name.as_bytes());
            lanes.lane(4, slot as u64, u64::from(entry.is_directory));
        }
    }
}

/// Path-indexed in-memory blob store with per-fd open-file and
/// open-directory tables.
///
/// File and directory fds share a single monotonic allocator; the
/// two open-tables stay distinct so `close_fd` on a dir fd (and
/// vice versa) surfaces as CELL_EBADF.
///
/// [`Default`] is implemented manually because a derived `Default`
/// would set `next_fd = 0`, violating the never-zero invariant.
#[derive(Debug, Clone)]
pub struct FsStore {
    /// Keyed by path; each blob's lane object is the path's digest.
    blobs: LaneMap<String, BlobEntry>,
    open_fds: LaneMap<u32, FdEntry>,
    open_dirs: LaneMap<u32, DirSnapshot>,
    next_fd: u32,
}

impl Default for FsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FsStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self {
            blobs: LaneMap::new_keyed_by_ref(source::FS_BLOB, |path: &String| {
                object_digest(path.as_bytes())
            }),
            open_fds: LaneMap::new(source::FS_FD, u64::from),
            open_dirs: LaneMap::new(source::FS_DIR, u64::from),
            next_fd: LV2_FS_OBJECT_ID_BASE,
        }
    }

    /// Register `bytes` under `path`. Single-write: the fd table
    /// stores paths, not blob snapshots, so a silent replacement
    /// would mutate bytes an open fd is mid-read on.
    ///
    /// # Errors
    ///
    /// - [`FsError::PathAlreadyRegistered`] if `path` already has
    ///   a blob.
    pub fn register_blob(&mut self, path: String, bytes: Vec<u8>) -> Result<(), FsError> {
        if self.blobs.contains_by(path.as_str()) {
            return Err(FsError::PathAlreadyRegistered);
        }
        let content_digest = object_digest(&bytes);
        self.blobs.insert(
            path,
            BlobEntry {
                bytes,
                content_digest,
            },
        );
        Ok(())
    }

    /// Host-side introspection only. Guest reads must go through
    /// [`Self::open_fd`] + [`Self::read_at`] so the sync partial sees
    /// the offset advance.
    pub fn lookup_blob(&self, path: &str) -> Option<&[u8]> {
        self.blobs.get_by(path).map(|b| b.bytes.as_slice())
    }

    /// Cheaper than [`Self::lookup_blob`] when the caller does not
    /// need the bytes; does not borrow the blob.
    pub fn has_path(&self, path: &str) -> bool {
        self.blobs.contains_by(path)
    }

    /// Whether the store has any registered blobs or open fds / dirs.
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty() && self.open_fds.is_empty() && self.open_dirs.is_empty()
    }

    /// Number of registered blobs.
    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }

    /// Number of currently-open fds.
    pub fn open_fd_count(&self) -> usize {
        self.open_fds.len()
    }

    /// Number of currently-open directory fds.
    pub fn open_dir_count(&self) -> usize {
        self.open_dirs.len()
    }

    /// Bumps the never-recycle counter only on success; `UnknownPath`
    /// does not burn the prior `next_fd` value.
    ///
    /// # Errors
    ///
    /// - [`FsError::UnknownPath`] if the blob is not registered.
    /// - [`FsError::FdExhausted`] if the allocator has handed out
    ///   the full `u32::MAX - LV2_FS_OBJECT_ID_BASE` fd range.
    pub fn open_fd(&mut self, path: &str) -> Result<u32, FsError> {
        if !self.blobs.contains_by(path) {
            return Err(FsError::UnknownPath);
        }
        let fd = self.next_fd;
        let next = fd.checked_add(1).ok_or(FsError::FdExhausted)?;
        self.next_fd = next;
        self.open_fds.insert(
            fd,
            FdEntry {
                path: path.to_owned(),
                offset: 0,
            },
        );
        Ok(fd)
    }

    /// Release the fd; subsequent ops on it return `UnknownFd`.
    pub fn close_fd(&mut self, fd: u32) -> Result<(), FsError> {
        self.open_fds
            .remove(fd)
            .map(|_| ())
            .ok_or(FsError::UnknownFd)
    }

    /// Read up to `max_bytes` from the fd's current offset, advancing
    /// the offset by the returned length. Returns an empty vector at
    /// EOF. A 0-byte read does not move the offset; only bytes
    /// actually returned advance the cursor.
    pub fn read_at(&mut self, fd: u32, max_bytes: usize) -> Result<Vec<u8>, FsError> {
        let mut entry = self.open_fds.get_mut(fd).ok_or(FsError::UnknownFd)?;
        let blob = self
            .blobs
            .get_by(entry.path.as_str())
            .ok_or(FsError::UnknownPath)?;
        let len = blob.bytes.len();
        // Clamp before the usize cast: a 32-bit host would otherwise
        // wrap a >4 GiB offset to a small in-range value.
        let start = entry.offset.min(len as u64) as usize;
        // `start + max_bytes` would overflow for huge `max_bytes`.
        let take = max_bytes.min(len.saturating_sub(start));
        let slice = blob.bytes[start..start + take].to_vec();
        entry.offset = entry.offset.saturating_add(take as u64);
        Ok(slice)
    }

    /// Returns the new absolute position. Seeks past EOF (but within
    /// u64) are allowed; the next read returns empty.
    ///
    /// # Errors
    ///
    /// - [`FsError::SeekOutOfRange`] when the result lands outside
    ///   `[0, u64::MAX]` (negative-past-zero or positive overflow).
    pub fn seek(&mut self, fd: u32, offset: i64, whence: SeekWhence) -> Result<u64, FsError> {
        let mut entry = self.open_fds.get_mut(fd).ok_or(FsError::UnknownFd)?;
        let size = self
            .blobs
            .get_by(entry.path.as_str())
            .ok_or(FsError::UnknownPath)?
            .bytes
            .len() as u64;
        let base = match whence {
            SeekWhence::Set => 0i128,
            SeekWhence::Cur => entry.offset as i128,
            SeekWhence::End => size as i128,
        };
        let new_pos = base + offset as i128;
        if !(0..=u64::MAX as i128).contains(&new_pos) {
            return Err(FsError::SeekOutOfRange);
        }
        let new_pos = new_pos as u64;
        entry.offset = new_pos;
        Ok(new_pos)
    }

    /// Path-based stat.
    pub fn stat_path(&self, path: &str) -> Result<FileStat, FsError> {
        let blob = self.blobs.get_by(path).ok_or(FsError::UnknownPath)?;
        Ok(FileStat {
            size: blob.bytes.len() as u64,
        })
    }

    /// Fd-based stat.
    pub fn fstat(&self, fd: u32) -> Result<FileStat, FsError> {
        let entry = self.open_fds.get(fd).ok_or(FsError::UnknownFd)?;
        let blob = self
            .blobs
            .get_by(entry.path.as_str())
            .ok_or(FsError::UnknownPath)?;
        Ok(FileStat {
            size: blob.bytes.len() as u64,
        })
    }

    /// Allocate a fresh directory fd over `entries`. The dispatcher
    /// owns ordering and filtering; FsStore walks entries in the
    /// order it received them.
    ///
    /// # Errors
    ///
    /// - [`FsError::FdExhausted`] if the allocator has handed out
    ///   the full `u32::MAX - LV2_FS_OBJECT_ID_BASE` fd range.
    pub fn open_dir(&mut self, entries: Vec<DirEntry>) -> Result<u32, FsError> {
        let fd = self.next_fd;
        let next = fd.checked_add(1).ok_or(FsError::FdExhausted)?;
        self.next_fd = next;
        self.open_dirs
            .insert(fd, DirSnapshot { entries, cursor: 0 });
        Ok(fd)
    }

    /// Return the next directory entry for `fd` and advance the
    /// cursor. Returns `Ok(None)` at EOF.
    ///
    /// # Errors
    ///
    /// - [`FsError::UnknownDir`] if `fd` is not an open directory.
    pub fn read_dir_entry(&mut self, fd: u32) -> Result<Option<DirEntry>, FsError> {
        let mut snap = self.open_dirs.get_mut(fd).ok_or(FsError::UnknownDir)?;
        if snap.cursor >= snap.entries.len() {
            return Ok(None);
        }
        let entry = snap.entries[snap.cursor].clone();
        snap.cursor += 1;
        Ok(Some(entry))
    }

    /// # Errors
    ///
    /// - [`FsError::UnknownDir`] if `fd` is not an open directory,
    ///   including the case where `fd` is a file fd.
    pub fn close_dir(&mut self, fd: u32) -> Result<(), FsError> {
        self.open_dirs
            .remove(fd)
            .map(|_| ())
            .ok_or(FsError::UnknownDir)
    }

    /// The store's partial of the sync-state sum: the blobs, the open
    /// file and directory fds, and the fd allocator's cursor.
    pub fn sync_partial(&self) -> u128 {
        self.blobs
            .partial()
            .wrapping_add(self.open_fds.partial())
            .wrapping_add(self.open_dirs.partial())
            .wrapping_add(self.next_fd_term())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.blobs
            .partial_from_scratch()
            .wrapping_add(self.open_fds.partial_from_scratch())
            .wrapping_add(self.open_dirs.partial_from_scratch())
            .wrapping_add(self.next_fd_term())
    }

    fn next_fd_term(&self) -> u128 {
        lanes::value_term(source::FS_NEXT_FD, 0, &self.next_fd)
    }
}

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod tests;
