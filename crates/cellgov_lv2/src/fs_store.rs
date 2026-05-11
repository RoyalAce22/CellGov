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

/// Monotonic fd allocator base. Also the lower bound of the FS-layer
/// fd range -- exposed crate-wide so dispatch tests can assert
/// `fd >= FD_BASE` rather than the weaker `fd != 0`.
pub(crate) const FD_BASE: u32 = 0x4000_0001;

/// Whence values for [`FsStore::seek`]. Matches PS3 `CELL_FS_SEEK_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekWhence {
    /// Offset measured from the start of the file.
    Set = 0,
    /// Offset measured from the current fd position.
    Cur = 1,
    /// Offset measured from the end of the file.
    End = 2,
}

impl SeekWhence {
    /// Decode a guest-supplied whence value. Returns `None` for any
    /// out-of-range value so the caller can map it to CELL_EINVAL.
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
///
/// `mtime` / `atime` / `ctime` are deterministic constants (zero in
/// the current implementation) -- the oracle has no concept of host
/// time, and content blobs are immutable so a real timestamp would
/// be misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    /// Size of the blob in bytes.
    pub size: u64,
}

/// Errors the FS layer surfaces. Dispatch handlers map these to
/// CELL_FS_* errno values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    /// Fd is not in the open-fd table.
    UnknownFd,
    /// Fd is not in the open-dir table. Distinct from `UnknownFd`
    /// so `close_fd` on a dir fd (or `close_dir` on a file fd)
    /// surfaces as CELL_EBADF without conflating the two
    /// allocators -- a guest that mixes them up should see the
    /// type mismatch surface, not silent success.
    UnknownDir,
    /// Path is not registered in the blob table.
    UnknownPath,
    /// Seek offset would land outside `[0, u64::MAX]`.
    SeekOutOfRange,
    /// Fd allocator hit `u32::MAX`. Hard to provoke in practice;
    /// reaching it would silently recycle fds (an aliased read
    /// from one open advances another open's offset), so the
    /// allocator surfaces it as an explicit error.
    FdExhausted,
    /// Manifest loader tried to register a second blob at a path
    /// that already has one. Registration is single-write so a
    /// later content swap cannot mutate bytes an open fd is
    /// reading from.
    PathAlreadyRegistered,
    /// A mount table entry tried to register a second mount with
    /// the same guest-path prefix. Mount tables are single-write
    /// per prefix to keep guest-to-host resolution unambiguous.
    MountAlreadyRegistered,
    /// A guest path tried to escape its mount via `..` segments.
    /// Surfaced as CELL_EACCES by the dispatch layer.
    PathTraversal,
}

/// One entry in a directory snapshot. The dispatcher captures
/// these from the host filesystem at `sys_fs_opendir` time and
/// hands them to [`FsStore::open_dir`]; `read_dir_entry` walks
/// them in registration order. Sorting is the dispatcher's
/// concern (the deterministic oracle requires lexicographic
/// order; FsStore preserves whatever order the dispatcher hands
/// over).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Filename (no path components). Must not contain `/` or
    /// embedded NUL; the dispatcher filters those before
    /// forwarding.
    pub name: String,
    /// Whether this entry is a sub-directory (vs a regular file).
    /// Symlinks and special files are filtered out at snapshot
    /// time so the dispatcher only ever forwards REGULAR or
    /// DIRECTORY shapes.
    pub is_directory: bool,
}

/// One read-only mount: a guest-path prefix served from a host
/// directory. Used by the dispatch layer when a guest path is not
/// pre-registered as a blob in [`FsStore`]. Examples:
/// `prefix = "/app_home"` mapped to the title's USRDIR;
/// `prefix = "/dev_hdd0"` mapped to a workspace dev_hdd0 mirror.
///
/// `prefix` is normalized at construction (no trailing `/`,
/// must start with `/`).
///
/// Read-only by design: writes / mkdir / unlink return CELL_EROFS
/// from the dispatch layer regardless of host-side permissions.
/// CellGov is a deterministic oracle, not a runtime emulator;
/// mutating host disk would leak host state into guest behavior.
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
    /// Returns `None` if `prefix` is empty, doesn't start with
    /// `/`, or contains `..`. The dispatch layer must surface
    /// these as configuration errors at boot, not runtime
    /// failures.
    pub fn new(prefix: impl Into<String>, host_root: PathBuf) -> Option<Self> {
        let mut prefix = prefix.into();
        if prefix.is_empty() || !prefix.starts_with('/') {
            return None;
        }
        // Reject `..` in the prefix itself; it would let a
        // resolution land outside any conceivable mount root.
        if prefix.split('/').any(|seg| seg == "..") {
            return None;
        }
        // Strip trailing slash so `prefix` and `prefix/` resolve
        // identically.
        while prefix.len() > 1 && prefix.ends_with('/') {
            prefix.pop();
        }
        Some(Self { prefix, host_root })
    }
}

/// Ordered set of [`FsMount`]s consulted by the dispatch layer
/// when a guest path is not registered as a blob. Mounts are
/// consulted in registration order; the first whose prefix
/// matches resolves the path.
#[derive(Debug, Clone, Default)]
pub struct FsMountTable {
    mounts: Vec<FsMount>,
}

impl FsMountTable {
    /// Empty mount table. Boot wires real mounts via
    /// [`Self::add`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a mount. Errors on duplicate prefix.
    ///
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
    /// Returns `Ok(Some(host_path))` when a mount matches.
    /// Returns `Ok(None)` when no mount matches (the dispatcher
    /// should surface this as ENOENT after consulting other
    /// stores).
    /// Returns `Err(FsError::PathTraversal)` when the resolved
    /// path would escape the mount root via `..` segments.
    ///
    /// Path normalization:
    ///
    /// - Empty segments (`//`) are skipped.
    /// - `.` segments are skipped (titles routinely emit
    ///   `/app_home/./Foo`).
    /// - `..` segments are rejected outright. A title that
    ///   genuinely needs `..` traversal is anti-scope; an
    ///   investigation should determine whether the title is
    ///   path-escaping by accident or by design.
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

/// Match `guest_path` against `prefix`. The match succeeds when
/// `guest_path` equals `prefix` exactly OR begins with
/// `prefix + '/'`; partial prefix matches like `/app_home` vs
/// `/app_homeFoo` are rejected.
///
/// The root mount `/` is handled specially: any guest path
/// starting with `/` matches, with the leading `/` stripped.
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
    /// Pre-computed content hash. Folded into [`FsStore::state_hash`]
    /// so a content swap surfaces as a determinism break, without
    /// re-hashing the full blob on every step.
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
    /// Index of the next entry to return from `read_dir_entry`.
    /// Equal to `entries.len()` at EOF.
    cursor: usize,
}

/// Path-indexed in-memory blob store with per-fd open-file and
/// open-directory tables.
///
/// `Default` is implemented manually rather than derived: a derived
/// `Default` produces `next_fd = 0` (the [`u32`] default), which
/// would silently hand out fd `0` on the first open and violate the
/// never-recycle / never-zero invariant. The manual impl forwards to
/// [`Self::new`] so both constructors start `next_fd` at `FD_BASE`.
///
/// File and directory fds share a single monotonic allocator. The
/// two open-tables are kept distinct so `close_fd` on a dir fd (and
/// vice versa) surfaces as CELL_EBADF rather than silently
/// succeeding -- a guest that mixes the two has a bug worth
/// surfacing, not papering over.
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

    /// Register `bytes` under `path`. Single-write: a second
    /// registration at the same path returns
    /// [`FsError::PathAlreadyRegistered`] and the existing bytes
    /// stay. Pinned by the open-fd contract -- the fd table stores
    /// paths, not blob snapshots, so a silent replacement would
    /// mutate bytes an open fd is mid-read on.
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

    /// Read-only lookup; `None` if no blob is registered at `path`.
    ///
    /// **Host-side introspection only.** Guest reads must go through
    /// the fd allocation path ([`Self::open_fd`] then
    /// [`Self::read_at`]); using `lookup_blob` to short-circuit the
    /// fd table would skip the offset advance and the state-hash
    /// contribution that the determinism contract depends on.
    pub fn lookup_blob(&self, path: &str) -> Option<&[u8]> {
        self.blobs.get(path).map(|b| b.bytes.as_slice())
    }

    /// Whether `path` is registered. Cheaper than `lookup_blob` when
    /// the caller does not need the bytes -- the dispatch layer's
    /// existence-then-flag precedence relies on this not borrowing
    /// the blob bytes.
    pub fn has_path(&self, path: &str) -> bool {
        self.blobs.contains_key(path)
    }

    /// Whether the store has any registered blobs or open fds /
    /// open dirs.
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty() && self.open_fds.is_empty() && self.open_dirs.is_empty()
    }

    /// Test-only: fast-forward the fd allocator. Used by dispatch
    /// tests that need to provoke [`FsError::FdExhausted`] without
    /// looping `open_fd` ~4 billion times. Crate-private so only
    /// in-crate tests can reach it.
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

    /// Allocate a fresh fd against `path`. Bumps the never-recycle
    /// counter only on success; the prior `next_fd` value is not
    /// burned by an `UnknownPath` error.
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
        // Advance first; on overflow, leave the table untouched so
        // the caller can choose to surface CELL_FS_EMFILE without
        // having silently recycled a live fd.
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

    /// Release the fd. Subsequent reads / stats / closes on the same
    /// fd return `UnknownFd`.
    pub fn close_fd(&mut self, fd: u32) -> Result<(), FsError> {
        self.open_fds
            .remove(&fd)
            .map(|_| ())
            .ok_or(FsError::UnknownFd)
    }

    /// Read up to `max_bytes` from the fd's current offset. Advances
    /// the offset by the returned slice's length. Returns an empty
    /// vector at EOF (caller maps to CELL_OK with bytes_read = 0).
    ///
    /// A 0-byte read does not move the offset, even when the offset
    /// is past EOF; only bytes actually returned advance the cursor.
    pub fn read_at(&mut self, fd: u32, max_bytes: usize) -> Result<Vec<u8>, FsError> {
        let entry = self.open_fds.get_mut(&fd).ok_or(FsError::UnknownFd)?;
        let blob = self.blobs.get(&entry.path).ok_or(FsError::UnknownPath)?;
        let len = blob.bytes.len();
        // Clamp first, THEN cast: `entry.offset as usize` would
        // truncate on 32-bit hosts and a guest seek to >4 GiB
        // would wrap to a small in-range value.
        let start = entry.offset.min(len as u64) as usize;
        // `len.saturating_sub(start)` is the bytes available; min
        // with `max_bytes` gives the bytes to take. Computing this
        // way avoids the `start + max_bytes` overflow that would
        // panic in debug or wrap in release for huge `max_bytes`.
        let take = max_bytes.min(len.saturating_sub(start));
        let slice = blob.bytes[start..start + take].to_vec();
        // Advance only by bytes actually returned, NOT by clamping
        // to EOF. A 0-byte read after seeking past EOF must leave
        // the offset where the seek left it.
        entry.offset = entry.offset.saturating_add(take as u64);
        Ok(slice)
    }

    /// Update the fd's offset. Returns the new absolute position.
    /// Seeks landing outside `[0, u64::MAX]` return
    /// [`FsError::SeekOutOfRange`]; seeks past EOF (but within u64)
    /// are allowed and the next read returns empty.
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
        // Symmetric range check: catch BOTH the negative-past-zero
        // case AND the positive-overflow case before the u64 cast.
        if !(0..=u64::MAX as i128).contains(&new_pos) {
            return Err(FsError::SeekOutOfRange);
        }
        let new_pos = new_pos as u64;
        entry.offset = new_pos;
        Ok(new_pos)
    }

    /// Path-based stat. `Err(UnknownPath)` if no blob is registered.
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
    /// is responsible for ordering (lexicographic byte order) and
    /// for filtering symlinks / special files; FsStore stores
    /// whatever it is handed and walks it in registration order.
    ///
    /// # Errors
    ///
    /// - [`FsError::FdExhausted`] if the allocator has handed out
    ///   the full `u32::MAX - FD_BASE` fd range.
    pub fn open_dir(&mut self, entries: Vec<DirEntry>) -> Result<u32, FsError> {
        let fd = self.next_fd;
        // Advance first; on overflow, leave the table untouched so
        // an exhausted allocator surfaces as EMFILE without having
        // silently recycled a live fd.
        let next = fd.checked_add(1).ok_or(FsError::FdExhausted)?;
        self.next_fd = next;
        self.open_dirs
            .insert(fd, DirSnapshot { entries, cursor: 0 });
        Ok(fd)
    }

    /// Return the next directory entry for `fd` and advance the
    /// cursor by one. Returns `Ok(None)` at EOF (the dispatcher
    /// maps that to a zero-bytes-written `nread` per the PS3 ABI).
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

    /// Release a directory fd. Subsequent reads / closes return
    /// `UnknownDir`.
    ///
    /// # Errors
    ///
    /// - [`FsError::UnknownDir`] if `fd` is not an open directory
    ///   (including the case where `fd` is a file fd; mixing the
    ///   two is a guest bug worth surfacing).
    pub fn close_dir(&mut self, fd: u32) -> Result<(), FsError> {
        self.open_dirs
            .remove(&fd)
            .map(|_| ())
            .ok_or(FsError::UnknownDir)
    }

    /// Determinism-stable hash of every observable piece of state:
    /// content hashes per registered path, current fd offsets, and
    /// the next-fd counter. Folded into [`crate::host::Lv2Host::state_hash`].
    ///
    /// Iteration order over `blobs` and `open_fds` is the
    /// [`BTreeMap`] sort order on the key. Swapping either map for
    /// a [`HashMap`] would make this hash insertion-order sensitive
    /// and two runs could disagree. The
    /// `state_hash_is_insertion_order_independent` test pins this
    /// for blobs.
    ///
    /// [`HashMap`]: std::collections::HashMap
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
        // Asking for more than available -> truncated to remaining.
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
        let _ = s.read_at(fd, 2).unwrap(); // offset = 2
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
        // Six paths -- forward vs reverse insertion. Two paths is a
        // coin flip on whether a hypothetical `HashMap` swap iterates
        // them in the same order; six is enough that a diverging
        // iteration order would almost certainly produce a different
        // hash, so the test would fail loudly under a map-type swap.
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
        // Original bytes preserved on rejection.
        assert_eq!(s.lookup_blob("/foo"), Some(b"first".as_slice()));
    }

    #[test]
    fn fd_exhaustion_at_u32_max_is_explicit() {
        let mut s = fs_with("/foo", b"x");
        // Fast-forward the allocator to one before the cap. The
        // checked_add bumps next_fd to u32::MAX after this open; the
        // following open finds nothing to hand out and errors. Last
        // valid fd is u32::MAX - 1 (u32::MAX is the exhaustion
        // sentinel).
        s.next_fd = u32::MAX - 1;
        let last = s.open_fd("/foo").unwrap();
        assert_eq!(last, u32::MAX - 1);
        assert_eq!(s.open_fd("/foo"), Err(FsError::FdExhausted));
        // Failed open did NOT silently insert; the table still has
        // exactly one entry.
        assert_eq!(s.open_fd_count(), 1);
    }

    #[test]
    fn unknown_path_open_does_not_burn_an_fd() {
        let mut s = fs_with("/foo", b"x");
        let h0 = s.state_hash();
        assert_eq!(s.open_fd("/missing"), Err(FsError::UnknownPath));
        // Failed open must not advance next_fd; otherwise UnknownPath
        // probes from the guest leak fd id space.
        assert_eq!(s.state_hash(), h0);
    }

    #[test]
    fn seek_positive_overflow_returns_out_of_range() {
        let mut s = fs_with("/foo", b"x");
        let fd = s.open_fd("/foo").unwrap();
        // Stage offset at u64::MAX via two compounding Cur seeks.
        // i64::MAX + i64::MAX = u64::MAX - 1; one more Cur step
        // hits u64::MAX exactly; the next overflows.
        assert_eq!(
            s.seek(fd, i64::MAX, SeekWhence::Set).unwrap(),
            i64::MAX as u64
        );
        assert_eq!(s.seek(fd, i64::MAX, SeekWhence::Cur).unwrap(), u64::MAX - 1);
        assert_eq!(s.seek(fd, 1, SeekWhence::Cur).unwrap(), u64::MAX);
        // One more positive step would wrap: must surface as
        // SeekOutOfRange, not silent wrap to small offset.
        assert_eq!(s.seek(fd, 1, SeekWhence::Cur), Err(FsError::SeekOutOfRange));
    }

    #[test]
    fn zero_byte_read_does_not_disturb_offset() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        s.seek(fd, 100, SeekWhence::Set).unwrap();
        let _ = s.read_at(fd, 0).unwrap();
        // Offset stayed at 100 -- a 0-byte read does not clamp to EOF.
        assert_eq!(s.seek(fd, 0, SeekWhence::Cur).unwrap(), 100);
    }

    #[test]
    fn huge_max_bytes_does_not_overflow_usize() {
        let mut s = fs_with("/foo", b"abc");
        let fd = s.open_fd("/foo").unwrap();
        // usize::MAX would have panicked / wrapped under the
        // pre-fix `start + max_bytes` calculation.
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
        // Open-then-close advances next_fd, so h2 differs from h0
        // even though no fds remain open. Pin: never-recycle property.
        assert_ne!(h0, h2);
        // And differs from the open state too.
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
        // File and dir fds share the monotonic allocator: dir fd
        // must be the next id, never coincide with a live file fd.
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
        // EOF -- subsequent reads return None without erroring.
        assert!(s.read_dir_entry(fd).unwrap().is_none());
        assert!(s.read_dir_entry(fd).unwrap().is_none());
    }

    #[test]
    fn read_dir_entry_unknown_fd_is_unknown_dir() {
        let mut s = FsStore::new();
        // Unallocated fd.
        assert_eq!(s.read_dir_entry(0xDEAD_BEEF), Err(FsError::UnknownDir));
        // File fd is not a dir fd: type-mixing must surface.
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
        // close_dir on a file fd is the type-mixing case; the
        // file fd stays open so close_fd still works on it.
        assert_eq!(s.close_dir(file_fd), Err(FsError::UnknownDir));
        assert!(s.close_fd(file_fd).is_ok());
    }

    #[test]
    fn close_fd_rejects_dir_fd() {
        let mut s = FsStore::new();
        let dir_fd = s.open_dir(vec![dir_entry("a", false)]).unwrap();
        // Symmetric: close_fd on a dir fd surfaces UnknownFd; the
        // dir fd is still releasable via close_dir.
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
        // Open then close advances next_fd, so h3 also differs
        // from h0 (never-recycle property is observable).
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
        // Bare prefix without trailing slash maps to the host
        // root itself (used by opendir on a mount root).
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
        // "/app_homeFoo" is NOT under /app_home; the match is
        // delimited by `/`, not byte prefix.
        let t = standard_table();
        assert_eq!(t.resolve("/app_homeFoo").unwrap(), None);
        assert_eq!(t.resolve("/app_homeFoo/bar").unwrap(), None);
    }

    #[test]
    fn resolve_picks_first_matching_mount() {
        // Registration order wins; the table is FIFO.
        let mut t = FsMountTable::new();
        t.add(FsMount::new("/app_home", PathBuf::from("/first")).unwrap())
            .unwrap();
        // Pretend a second mount could match (e.g. a more
        // generic root); this guards future re-orderings.
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
        // Mount at "/" is unusual but supported (HostRoot opt-in).
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
