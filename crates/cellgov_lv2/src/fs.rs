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

/// Path-indexed in-memory blob store with per-fd open-file table.
///
/// `Default` is implemented manually rather than derived: a derived
/// `Default` produces `next_fd = 0` (the [`u32`] default), which
/// would silently hand out fd `0` on the first open and violate the
/// never-recycle / never-zero invariant. The manual impl forwards to
/// [`Self::new`] so both constructors start `next_fd` at `FD_BASE`.
#[derive(Debug, Clone)]
pub struct FsStore {
    blobs: BTreeMap<String, BlobEntry>,
    open_fds: BTreeMap<u32, FdEntry>,
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

    /// Whether the store has any registered blobs or open fds.
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty() && self.open_fds.is_empty()
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
}
