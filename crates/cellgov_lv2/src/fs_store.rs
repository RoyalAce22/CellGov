//! In-memory filesystem layer.
//!
//! [`FsStore`] is host-side data populated at boot from a per-title
//! content manifest and consumed by the `sys_fs_*` LV2 dispatch
//! handlers. It is the back end for guest reads from paths a title
//! expects to load (`/app_home/Data/.../*.xml`, configuration files,
//! etc.); cellgov has no real filesystem so the bytes come from the
//! title manifest's `[content]` block.
//!
//! A blob is registered once at boot and is immutable thereafter.
//! Each `sys_fs_open` allocates a fresh fd whose offset advances
//! independently per fd; two opens of the same path each see the
//! file from byte 0.
//!
//! All ids are deterministic: fds come from a monotonic counter that
//! never recycles within a boot.

use std::collections::BTreeMap;
use std::path::PathBuf;

use cellgov_mem::Fnv1aHasher;

/// Monotonic fd allocator base. Matches real PS3's `lv2_fs_object`
/// `id_base = 3` (per `rpcs3/Emu/Cell/lv2/sys_fs.h`): file fds are
/// small ints in `[3, 255)` on real PS3. PSL1GHT-built titles encode
/// the fd into narrow struct fields and load it with `lbz`/`lhz`/`lwz`
/// semantics that truncate high bits; returning fds in the billions
/// corrupts the fd in the title's internal table.
pub(crate) const FD_BASE: u32 = 3;

/// Whence values for [`FsStore::seek`]. Matches PS3 `CELL_FS_SEEK_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        match value {
            0 => Some(Self::Set),
            1 => Some(Self::Cur),
            2 => Some(Self::End),
            _ => None,
        }
    }
}

/// File-stat shape returned by [`FsStore::stat_path`] / [`FsStore::fstat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    /// Blob size in bytes.
    pub size: u64,
}

/// Errors the FS layer surfaces. Dispatch handlers map these to
/// CELL_FS_* errno values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    /// Fd is not in the open-fd table.
    UnknownFd,
    /// Distinct from `UnknownFd` so `close_fd` on a dir fd (or
    /// `close_dir` on a file fd) surfaces as CELL_EBADF rather than
    /// silent success.
    UnknownDir,
    /// Path is not registered in the blob table.
    UnknownPath,
    /// Seek offset would land outside `[0, u64::MAX]`.
    SeekOutOfRange,
    /// Fd allocator hit `u32::MAX`. Surfaced rather than recycled
    /// so an aliased read cannot advance another open's offset.
    FdExhausted,
    /// Registration is single-write per path so a later content swap
    /// cannot mutate bytes an open fd is reading from.
    PathAlreadyRegistered,
    /// Single-write per prefix to keep guest-to-host resolution
    /// unambiguous.
    MountAlreadyRegistered,
    /// Surfaced as CELL_EACCES by the dispatch layer.
    PathTraversal,
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

/// One read-only mount: a guest-path prefix served from a host
/// directory.
///
/// `prefix` is normalized at construction (no trailing `/`, must
/// start with `/`). Writes / mkdir / unlink return CELL_EROFS from
/// the dispatch layer regardless of host-side permissions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsMount {
    /// Guest path prefix, e.g. `/app_home`. No trailing slash.
    pub prefix: String,
    /// Host directory backing the mount.
    pub host_root: PathBuf,
}

impl FsMount {
    /// Build a mount, normalizing `prefix`.
    ///
    /// # Errors
    ///
    /// Returns `None` if `prefix` is empty, doesn't start with `/`,
    /// or contains `..`.
    pub fn new(prefix: impl Into<String>, host_root: PathBuf) -> Option<Self> {
        let mut prefix = prefix.into();
        if prefix.is_empty() || !prefix.starts_with('/') {
            return None;
        }
        if prefix.split('/').any(|seg| seg == "..") {
            return None;
        }
        while prefix.len() > 1 && prefix.ends_with('/') {
            prefix.pop();
        }
        Some(Self { prefix, host_root })
    }
}

/// Ordered set of [`FsMount`]s.
///
/// Mounts are consulted in registration order; the first whose
/// prefix matches resolves the path.
#[derive(Debug, Clone, Default)]
pub struct FsMountTable {
    mounts: Vec<FsMount>,
}

impl FsMountTable {
    /// Empty mount table.
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    ///
    /// - [`FsError::MountAlreadyRegistered`] if a mount with the
    ///   same prefix is already in the table.
    pub fn add(&mut self, mount: FsMount) -> Result<(), FsError> {
        if self.mounts.iter().any(|m| m.prefix == mount.prefix) {
            return Err(FsError::MountAlreadyRegistered);
        }
        self.mounts.push(mount);
        Ok(())
    }

    /// Resolve a guest path to a host path.
    ///
    /// Normalizes empty segments (`//`) and `.` segments; rejects
    /// `..` segments as [`FsError::PathTraversal`].
    ///
    /// # Errors
    ///
    /// - [`FsError::PathTraversal`] when the resolved path would
    ///   escape the mount root via `..` segments.
    pub fn resolve(&self, guest_path: &str) -> Result<Option<PathBuf>, FsError> {
        for mount in &self.mounts {
            let Some(rest) = strip_mount_prefix(guest_path, &mount.prefix) else {
                continue;
            };
            let mut host = mount.host_root.clone();
            for segment in rest.split('/') {
                if segment.is_empty() || segment == "." {
                    continue;
                }
                if segment == ".." {
                    return Err(FsError::PathTraversal);
                }
                host.push(segment);
            }
            return Ok(Some(host));
        }
        Ok(None)
    }

    /// Iterate registered mounts in registration order.
    pub fn mounts(&self) -> impl Iterator<Item = &FsMount> {
        self.mounts.iter()
    }
}

/// Match `guest_path` against `prefix`, succeeding on exact match
/// or `prefix + '/'`. The root mount `/` matches any path starting
/// with `/`, stripping the leading slash.
fn strip_mount_prefix<'a>(guest_path: &'a str, prefix: &str) -> Option<&'a str> {
    if guest_path == prefix {
        return Some("");
    }
    if prefix == "/" {
        return guest_path.strip_prefix('/');
    }
    let with_slash_len = prefix.len() + 1;
    if guest_path.len() >= with_slash_len
        && guest_path.starts_with(prefix)
        && guest_path.as_bytes()[prefix.len()] == b'/'
    {
        Some(&guest_path[with_slash_len..])
    } else {
        None
    }
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
            next_fd: FD_BASE,
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

    /// Fast-forward the fd allocator to provoke
    /// [`FsError::FdExhausted`] without looping `open_fd` ~4 billion
    /// times.
    #[cfg(test)]
    pub(crate) fn force_next_fd_for_test(&mut self, value: u32) {
        self.next_fd = value;
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
    ///   the full `u32::MAX - FD_BASE` fd range.
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
    ///   the full `u32::MAX - FD_BASE` fd range.
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
mod tests {
    use super::*;

    fn fs_with(path: &str, bytes: &[u8]) -> FsStore {
        let mut s = FsStore::new();
        s.register_blob(path.to_owned(), bytes.to_vec()).unwrap();
        s
    }

    #[test]
    fn lookup_returns_registered_blob() {
        let s = fs_with("/foo", b"hello");
        assert_eq!(s.lookup_blob("/foo"), Some(b"hello".as_slice()));
        assert_eq!(s.lookup_blob("/bar"), None);
    }

    #[test]
    fn open_unknown_path_returns_unknown_path() {
        let mut s = FsStore::new();
        assert_eq!(s.open_fd("/missing"), Err(FsError::UnknownPath));
    }

    #[test]
    fn fd_allocator_is_monotonic_and_distinct_per_open() {
        let mut s = fs_with("/foo", b"x");
        let a = s.open_fd("/foo").unwrap();
        let b = s.open_fd("/foo").unwrap();
        assert_eq!(a, FD_BASE);
        assert_eq!(b, FD_BASE + 1);
        assert_ne!(a, b);
    }

    #[test]
    fn read_advances_offset_and_returns_next_slice() {
        let mut s = fs_with("/foo", b"abcdef");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.read_at(fd, 3).unwrap(), b"abc");
        assert_eq!(s.read_at(fd, 3).unwrap(), b"def");
    }

    #[test]
    fn read_at_eof_returns_empty() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        let _ = s.read_at(fd, 3).unwrap();
        assert!(s.read_at(fd, 3).unwrap().is_empty());
    }

    #[test]
    fn read_clamps_to_remaining_bytes() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.read_at(fd, 100).unwrap(), b"abc");
    }

    #[test]
    fn close_releases_fd_and_subsequent_ops_fail() {
        let mut s = fs_with("/foo", b"x");
        let fd = s.open_fd("/foo").unwrap();
        assert!(s.close_fd(fd).is_ok());
        assert_eq!(s.close_fd(fd), Err(FsError::UnknownFd));
        assert_eq!(s.read_at(fd, 1), Err(FsError::UnknownFd));
        assert_eq!(s.fstat(fd), Err(FsError::UnknownFd));
    }

    #[test]
    fn seek_set_jumps_to_absolute_offset() {
        let mut s = fs_with("/foo", b"abcdef");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.seek(fd, 4, SeekWhence::Set).unwrap(), 4);
        assert_eq!(s.read_at(fd, 10).unwrap(), b"ef");
    }

    #[test]
    fn seek_cur_advances_relative() {
        let mut s = fs_with("/foo", b"abcdef");
        let fd = s.open_fd("/foo").unwrap();
        let _ = s.read_at(fd, 2).unwrap();
        assert_eq!(s.seek(fd, 2, SeekWhence::Cur).unwrap(), 4);
        assert_eq!(s.read_at(fd, 10).unwrap(), b"ef");
    }

    #[test]
    fn seek_end_returns_size() {
        let mut s = fs_with("/foo", b"abcdef");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.seek(fd, 0, SeekWhence::End).unwrap(), 6);
    }

    #[test]
    fn seek_negative_past_zero_returns_out_of_range() {
        let mut s = fs_with("/foo", b"abcdef");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(
            s.seek(fd, -1, SeekWhence::Set),
            Err(FsError::SeekOutOfRange)
        );
    }

    #[test]
    fn seek_past_end_is_allowed_and_subsequent_read_returns_empty() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.seek(fd, 100, SeekWhence::Set).unwrap(), 100);
        assert!(s.read_at(fd, 10).unwrap().is_empty());
    }

    #[test]
    fn stat_path_returns_size() {
        let s = fs_with("/foo", b"hello world");
        assert_eq!(s.stat_path("/foo").unwrap().size, 11);
    }

    #[test]
    fn stat_unknown_path_returns_unknown_path() {
        let s = FsStore::new();
        assert_eq!(s.stat_path("/foo"), Err(FsError::UnknownPath));
    }

    #[test]
    fn fstat_returns_size_for_open_fd() {
        let mut s = fs_with("/foo", b"hello");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.fstat(fd).unwrap().size, 5);
    }

    #[test]
    fn whence_decoder_rejects_out_of_range() {
        assert_eq!(SeekWhence::from_guest(0), Some(SeekWhence::Set));
        assert_eq!(SeekWhence::from_guest(1), Some(SeekWhence::Cur));
        assert_eq!(SeekWhence::from_guest(2), Some(SeekWhence::End));
        assert_eq!(SeekWhence::from_guest(3), None);
        assert_eq!(SeekWhence::from_guest(0xFFFF_FFFF), None);
    }

    #[test]
    fn state_hash_changes_on_blob_registration() {
        let s0 = FsStore::new();
        let mut s1 = FsStore::new();
        s1.register_blob("/foo".into(), b"x".to_vec()).unwrap();
        assert_ne!(s0.state_hash(), s1.state_hash());
    }

    #[test]
    fn state_hash_changes_on_content_swap() {
        let mut s1 = FsStore::new();
        s1.register_blob("/foo".into(), b"x".to_vec()).unwrap();
        let h1 = s1.state_hash();
        let mut s2 = FsStore::new();
        s2.register_blob("/foo".into(), b"y".to_vec()).unwrap();
        let h2 = s2.state_hash();
        assert_ne!(h1, h2);
    }

    #[test]
    fn state_hash_is_insertion_order_independent() {
        let paths = ["/z", "/a", "/m", "/b", "/y", "/c"];
        let mut a = FsStore::new();
        for p in paths {
            a.register_blob(p.into(), p.as_bytes().to_vec()).unwrap();
        }
        let mut b = FsStore::new();
        for p in paths.iter().rev() {
            b.register_blob((*p).into(), p.as_bytes().to_vec()).unwrap();
        }
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn duplicate_register_returns_path_already_registered() {
        let mut s = FsStore::new();
        s.register_blob("/foo".into(), b"first".to_vec()).unwrap();
        assert_eq!(
            s.register_blob("/foo".into(), b"second".to_vec()),
            Err(FsError::PathAlreadyRegistered),
        );
        assert_eq!(s.lookup_blob("/foo"), Some(b"first".as_slice()));
    }

    #[test]
    fn fd_exhaustion_at_u32_max_is_explicit() {
        let mut s = fs_with("/foo", b"x");
        s.next_fd = u32::MAX - 1;
        let last = s.open_fd("/foo").unwrap();
        assert_eq!(last, u32::MAX - 1);
        assert_eq!(s.open_fd("/foo"), Err(FsError::FdExhausted));
        assert_eq!(s.open_fd_count(), 1);
    }

    #[test]
    fn unknown_path_open_does_not_burn_an_fd() {
        let mut s = fs_with("/foo", b"x");
        let h0 = s.state_hash();
        assert_eq!(s.open_fd("/missing"), Err(FsError::UnknownPath));
        assert_eq!(s.state_hash(), h0);
    }

    #[test]
    fn seek_positive_overflow_returns_out_of_range() {
        let mut s = fs_with("/foo", b"x");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(
            s.seek(fd, i64::MAX, SeekWhence::Set).unwrap(),
            i64::MAX as u64
        );
        assert_eq!(s.seek(fd, i64::MAX, SeekWhence::Cur).unwrap(), u64::MAX - 1);
        assert_eq!(s.seek(fd, 1, SeekWhence::Cur).unwrap(), u64::MAX);
        assert_eq!(s.seek(fd, 1, SeekWhence::Cur), Err(FsError::SeekOutOfRange));
    }

    #[test]
    fn zero_byte_read_does_not_disturb_offset() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        s.seek(fd, 100, SeekWhence::Set).unwrap();
        let _ = s.read_at(fd, 0).unwrap();
        assert_eq!(s.seek(fd, 0, SeekWhence::Cur).unwrap(), 100);
    }

    #[test]
    fn huge_max_bytes_does_not_overflow_usize() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.read_at(fd, usize::MAX).unwrap(), b"abc");
    }

    #[test]
    fn empty_blob_open_read_stat() {
        let mut s = fs_with("/empty", b"");
        let fd = s.open_fd("/empty").unwrap();
        assert!(s.read_at(fd, 100).unwrap().is_empty());
        assert_eq!(s.fstat(fd).unwrap().size, 0);
        assert_eq!(s.stat_path("/empty").unwrap().size, 0);
        assert_eq!(s.seek(fd, 0, SeekWhence::End).unwrap(), 0);
    }

    #[test]
    fn state_hash_changes_on_fd_offset_advance() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        let h0 = s.state_hash();
        let _ = s.read_at(fd, 1).unwrap();
        let h1 = s.state_hash();
        assert_ne!(h0, h1);
    }

    #[test]
    fn state_hash_changes_on_open_close_pair() {
        let mut s = fs_with("/foo", b"x");
        let h0 = s.state_hash();
        let fd = s.open_fd("/foo").unwrap();
        let h1 = s.state_hash();
        assert_ne!(h0, h1);
        s.close_fd(fd).unwrap();
        let h2 = s.state_hash();
        // Pins the never-recycle property: next_fd advance is
        // observable even after the fd is closed.
        assert_ne!(h0, h2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn empty_store_is_empty() {
        let s = FsStore::new();
        assert!(s.is_empty());
        assert_eq!(s.blob_count(), 0);
        assert_eq!(s.open_fd_count(), 0);
    }

    #[test]
    fn store_with_blob_is_not_empty() {
        let s = fs_with("/foo", b"x");
        assert!(!s.is_empty());
        assert_eq!(s.blob_count(), 1);
    }

    fn dir_entry(name: &str, is_directory: bool) -> DirEntry {
        DirEntry {
            name: name.to_owned(),
            is_directory,
        }
    }

    #[test]
    fn open_dir_returns_distinct_fds_sharing_file_allocator() {
        let mut s = FsStore::new();
        s.register_blob("/foo".into(), b"x".to_vec()).unwrap();
        let file_fd = s.open_fd("/foo").unwrap();
        let dir_fd = s.open_dir(vec![dir_entry("a", false)]).unwrap();
        assert_eq!(dir_fd, file_fd + 1);
        assert_ne!(dir_fd, file_fd);
    }

    #[test]
    fn read_dir_entry_walks_in_registration_order() {
        let mut s = FsStore::new();
        let entries = vec![
            dir_entry("a.xml", false),
            dir_entry("b.xml", false),
            dir_entry("sub", true),
        ];
        let fd = s.open_dir(entries).unwrap();
        let e0 = s.read_dir_entry(fd).unwrap().unwrap();
        assert_eq!(e0.name, "a.xml");
        assert!(!e0.is_directory);
        let e1 = s.read_dir_entry(fd).unwrap().unwrap();
        assert_eq!(e1.name, "b.xml");
        let e2 = s.read_dir_entry(fd).unwrap().unwrap();
        assert_eq!(e2.name, "sub");
        assert!(e2.is_directory);
        assert!(s.read_dir_entry(fd).unwrap().is_none());
        assert!(s.read_dir_entry(fd).unwrap().is_none());
    }

    #[test]
    fn read_dir_entry_unknown_fd_is_unknown_dir() {
        let mut s = FsStore::new();
        assert_eq!(s.read_dir_entry(0xDEAD_BEEF), Err(FsError::UnknownDir));
        s.register_blob("/foo".into(), b"x".to_vec()).unwrap();
        let file_fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.read_dir_entry(file_fd), Err(FsError::UnknownDir));
    }

    #[test]
    fn close_dir_releases_fd_and_subsequent_ops_fail() {
        let mut s = FsStore::new();
        let fd = s.open_dir(vec![dir_entry("a", false)]).unwrap();
        assert!(s.close_dir(fd).is_ok());
        assert_eq!(s.close_dir(fd), Err(FsError::UnknownDir));
        assert_eq!(s.read_dir_entry(fd), Err(FsError::UnknownDir));
    }

    #[test]
    fn close_dir_rejects_file_fd() {
        let mut s = FsStore::new();
        s.register_blob("/foo".into(), b"x".to_vec()).unwrap();
        let file_fd = s.open_fd("/foo").unwrap();
        assert_eq!(s.close_dir(file_fd), Err(FsError::UnknownDir));
        assert!(s.close_fd(file_fd).is_ok());
    }

    #[test]
    fn close_fd_rejects_dir_fd() {
        let mut s = FsStore::new();
        let dir_fd = s.open_dir(vec![dir_entry("a", false)]).unwrap();
        assert_eq!(s.close_fd(dir_fd), Err(FsError::UnknownFd));
        assert!(s.close_dir(dir_fd).is_ok());
    }

    #[test]
    fn open_dir_with_empty_entries_immediately_eofs() {
        let mut s = FsStore::new();
        let fd = s.open_dir(Vec::new()).unwrap();
        assert!(s.read_dir_entry(fd).unwrap().is_none());
    }

    #[test]
    fn state_hash_changes_on_dir_open_and_advance() {
        let mut s = FsStore::new();
        let h0 = s.state_hash();
        let fd = s
            .open_dir(vec![dir_entry("a", false), dir_entry("b", false)])
            .unwrap();
        let h1 = s.state_hash();
        assert_ne!(h0, h1, "open_dir must contribute to state_hash");
        let _ = s.read_dir_entry(fd).unwrap();
        let h2 = s.state_hash();
        assert_ne!(h1, h2, "advancing the cursor must contribute");
        s.close_dir(fd).unwrap();
        let h3 = s.state_hash();
        assert_ne!(h2, h3);
        assert_ne!(h0, h3);
    }
}

#[cfg(test)]
mod mount_tests {
    use super::*;

    fn standard_table() -> FsMountTable {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/app_home", PathBuf::from("/host/app")).unwrap())
            .unwrap();
        t.add(FsMount::new("/dev_hdd0", PathBuf::from("/host/hdd0")).unwrap())
            .unwrap();
        t
    }

    #[test]
    fn resolve_simple_app_home() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/Data/first.xml").unwrap(),
            Some(PathBuf::from("/host/app/Data/first.xml"))
        );
    }

    #[test]
    fn resolve_strips_dot_segments() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/./Data/./first.xml").unwrap(),
            Some(PathBuf::from("/host/app/Data/first.xml"))
        );
    }

    #[test]
    fn resolve_collapses_double_slashes() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home//Data//first.xml").unwrap(),
            Some(PathBuf::from("/host/app/Data/first.xml"))
        );
    }

    #[test]
    fn resolve_rejects_dotdot_traversal() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/../etc/passwd"),
            Err(FsError::PathTraversal)
        );
        assert_eq!(
            t.resolve("/app_home/Data/../../etc/passwd"),
            Err(FsError::PathTraversal)
        );
    }

    #[test]
    fn resolve_returns_none_for_no_mount() {
        let t = standard_table();
        assert_eq!(t.resolve("/dev_flash/foo").unwrap(), None);
    }

    #[test]
    fn resolve_handles_exact_prefix_match() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home").unwrap(),
            Some(PathBuf::from("/host/app"))
        );
    }

    #[test]
    fn resolve_handles_prefix_with_trailing_slash() {
        let t = standard_table();
        assert_eq!(
            t.resolve("/app_home/").unwrap(),
            Some(PathBuf::from("/host/app"))
        );
    }

    #[test]
    fn resolve_partial_prefix_does_not_match() {
        let t = standard_table();
        assert_eq!(t.resolve("/app_homeFoo").unwrap(), None);
        assert_eq!(t.resolve("/app_homeFoo/bar").unwrap(), None);
    }

    #[test]
    fn resolve_picks_first_matching_mount() {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/app_home", PathBuf::from("/first")).unwrap())
            .unwrap();
        t.add(FsMount::new("/app_home_alt", PathBuf::from("/second")).unwrap())
            .unwrap();
        assert_eq!(
            t.resolve("/app_home/x").unwrap(),
            Some(PathBuf::from("/first/x"))
        );
        assert_eq!(
            t.resolve("/app_home_alt/x").unwrap(),
            Some(PathBuf::from("/second/x"))
        );
    }

    #[test]
    fn add_rejects_duplicate_prefix() {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/app_home", PathBuf::from("/a")).unwrap())
            .unwrap();
        let err = t
            .add(FsMount::new("/app_home", PathBuf::from("/b")).unwrap())
            .unwrap_err();
        assert_eq!(err, FsError::MountAlreadyRegistered);
    }

    #[test]
    fn mount_new_normalizes_trailing_slash() {
        let m = FsMount::new("/app_home/", PathBuf::from("/x")).unwrap();
        assert_eq!(m.prefix, "/app_home");
    }

    #[test]
    fn mount_new_rejects_relative_prefix() {
        assert!(FsMount::new("app_home", PathBuf::from("/x")).is_none());
        assert!(FsMount::new("", PathBuf::from("/x")).is_none());
    }

    #[test]
    fn mount_new_rejects_dotdot_in_prefix() {
        assert!(FsMount::new("/app_home/..", PathBuf::from("/x")).is_none());
        assert!(FsMount::new("/../etc", PathBuf::from("/x")).is_none());
    }

    #[test]
    fn empty_table_resolves_nothing() {
        let t = FsMountTable::new();
        assert_eq!(t.resolve("/app_home/foo").unwrap(), None);
        assert_eq!(t.resolve("/").unwrap(), None);
    }

    #[test]
    fn mounts_iterates_in_registration_order() {
        let t = standard_table();
        let prefixes: Vec<&str> = t.mounts().map(|m| m.prefix.as_str()).collect();
        assert_eq!(prefixes, vec!["/app_home", "/dev_hdd0"]);
    }

    #[test]
    fn resolve_root_mount_with_subpath() {
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/", PathBuf::from("/host")).unwrap())
            .unwrap();
        assert_eq!(t.resolve("/").unwrap(), Some(PathBuf::from("/host")));
        assert_eq!(
            t.resolve("/foo/bar").unwrap(),
            Some(PathBuf::from("/host/foo/bar"))
        );
    }
}
