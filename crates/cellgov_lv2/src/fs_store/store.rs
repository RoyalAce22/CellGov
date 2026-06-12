use std::collections::BTreeMap;

use cellgov_mem::Fnv1aHasher;
use cellgov_ps3_abi::sys_fs::LV2_FS_OBJECT_ID_BASE;
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
    /// Pre-computed so [`FsStore::state_hash`] does not re-hash the
    /// full blob on every step.
    content_hash: u64,
}

#[derive(Debug, Clone)]
struct FdEntry {
    path: String,
    offset: u64,
}

#[derive(Debug, Clone)]
struct DirSnapshot {
    /// Frozen at `open_dir` time; never re-read from disk.
    entries: Vec<DirEntry>,
    /// Equal to `entries.len()` at EOF.
    cursor: usize,
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
    blobs: BTreeMap<String, BlobEntry>,
    open_fds: BTreeMap<u32, FdEntry>,
    open_dirs: BTreeMap<u32, DirSnapshot>,
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
            blobs: BTreeMap::new(),
            open_fds: BTreeMap::new(),
            open_dirs: BTreeMap::new(),
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
        if self.blobs.contains_key(&path) {
            return Err(FsError::PathAlreadyRegistered);
        }
        let mut hasher = Fnv1aHasher::new();
        hasher.write(&(bytes.len() as u64).to_le_bytes());
        hasher.write(&bytes);
        let content_hash = hasher.finish();
        self.blobs.insert(
            path,
            BlobEntry {
                bytes,
                content_hash,
            },
        );
        Ok(())
    }

    /// Host-side introspection only. Guest reads must go through
    /// [`Self::open_fd`] + [`Self::read_at`] so the offset advance
    /// and state-hash contribution are observable.
    pub fn lookup_blob(&self, path: &str) -> Option<&[u8]> {
        self.blobs.get(path).map(|b| b.bytes.as_slice())
    }

    /// Cheaper than [`Self::lookup_blob`] when the caller does not
    /// need the bytes; does not borrow the blob.
    pub fn has_path(&self, path: &str) -> bool {
        self.blobs.contains_key(path)
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
        if !self.blobs.contains_key(path) {
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
            .remove(&fd)
            .map(|_| ())
            .ok_or(FsError::UnknownFd)
    }

    /// Read up to `max_bytes` from the fd's current offset, advancing
    /// the offset by the returned length. Returns an empty vector at
    /// EOF. A 0-byte read does not move the offset; only bytes
    /// actually returned advance the cursor.
    pub fn read_at(&mut self, fd: u32, max_bytes: usize) -> Result<Vec<u8>, FsError> {
        let entry = self.open_fds.get_mut(&fd).ok_or(FsError::UnknownFd)?;
        let blob = self.blobs.get(&entry.path).ok_or(FsError::UnknownPath)?;
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
        let entry = self.open_fds.get_mut(&fd).ok_or(FsError::UnknownFd)?;
        let size = self
            .blobs
            .get(&entry.path)
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
        let blob = self.blobs.get(path).ok_or(FsError::UnknownPath)?;
        Ok(FileStat {
            size: blob.bytes.len() as u64,
        })
    }

    /// Fd-based stat.
    pub fn fstat(&self, fd: u32) -> Result<FileStat, FsError> {
        let entry = self.open_fds.get(&fd).ok_or(FsError::UnknownFd)?;
        let blob = self.blobs.get(&entry.path).ok_or(FsError::UnknownPath)?;
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
        let snap = self.open_dirs.get_mut(&fd).ok_or(FsError::UnknownDir)?;
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
            .remove(&fd)
            .map(|_| ())
            .ok_or(FsError::UnknownDir)
    }

    /// Determinism-stable hash of content per path, current fd
    /// offsets, and the next-fd counter. Folded into
    /// [`crate::host::Lv2Host::state_hash`].
    ///
    /// Iteration uses [`BTreeMap`] sort order; the
    /// `state_hash_is_insertion_order_independent` test pins this.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = Fnv1aHasher::new();
        hasher.write(&(self.blobs.len() as u64).to_le_bytes());
        for (path, entry) in &self.blobs {
            hasher.write(&(path.len() as u64).to_le_bytes());
            hasher.write(path.as_bytes());
            hasher.write(&entry.content_hash.to_le_bytes());
        }
        hasher.write(&(self.open_fds.len() as u64).to_le_bytes());
        for (fd, entry) in &self.open_fds {
            hasher.write(&fd.to_le_bytes());
            hasher.write(&(entry.path.len() as u64).to_le_bytes());
            hasher.write(entry.path.as_bytes());
            hasher.write(&entry.offset.to_le_bytes());
        }
        hasher.write(&(self.open_dirs.len() as u64).to_le_bytes());
        for (fd, snap) in &self.open_dirs {
            hasher.write(&fd.to_le_bytes());
            hasher.write(&(snap.cursor as u64).to_le_bytes());
            hasher.write(&(snap.entries.len() as u64).to_le_bytes());
            for entry in &snap.entries {
                hasher.write(&(entry.name.len() as u64).to_le_bytes());
                hasher.write(entry.name.as_bytes());
                hasher.write(&[u8::from(entry.is_directory)]);
            }
        }
        hasher.write(&self.next_fd.to_le_bytes());
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/store_tests.rs"]
mod tests;
