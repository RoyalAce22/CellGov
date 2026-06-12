//! Path-keyed SPU image registry returning monotonic non-zero handles.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

/// Monotonic host-side token for a loaded SPU image. Non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpuImageHandle(NonZeroU32);

impl SpuImageHandle {
    /// Wrap a raw handle value. Returns `None` if `raw == 0`.
    #[inline]
    pub const fn new(raw: u32) -> Option<Self> {
        match NonZeroU32::new(raw) {
            Some(nz) => Some(Self(nz)),
            None => None,
        }
    }

    /// Underlying non-zero handle value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0.get()
    }
}

/// A registered SPU image.
#[derive(Debug, Clone)]
pub struct SpuImageRecord {
    /// Allocated at registration time; non-zero.
    pub handle: SpuImageHandle,
    /// Full ELF bytes, not just loadable segments.
    pub elf_bytes: Vec<u8>,
}

/// Path-keyed store for registered SPU images.
///
/// # Invariants
/// - `by_path` and `by_handle` agree: every handle in either map
///   resolves through both. Debug-asserts in `register`,
///   `lookup_by_handle`, `len`, and `is_empty` guard the pairing.
/// - No host filesystem access; lookup is byte-exact, so `/a.elf`,
///   `/a.elf/`, and `//a.elf` are three distinct entries.
#[derive(Debug, Clone)]
pub struct ContentStore {
    by_path: BTreeMap<Vec<u8>, SpuImageRecord>,
    by_handle: BTreeMap<SpuImageHandle, Vec<u8>>,
    next_handle: u32,
    /// Cumulative count of [`Self::register`] invocations. Audit
    /// C-5a witness: the path-shape `debug_assert!`s in `register`
    /// (lines 74/79, 107/112/128 of this file) are conditional on
    /// register being called; this counter makes their silence
    /// non-vacuous. Increments per call regardless of whether the
    /// registration was novel or matched an existing path. Not
    /// snapshot/restore-captured: instrument state only.
    register_invocations: u64,
}

impl Default for ContentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self {
            by_path: BTreeMap::new(),
            by_handle: BTreeMap::new(),
            next_handle: 1,
            register_invocations: 0,
        }
    }

    /// Audit C-5a witness: cumulative count of `register` calls.
    /// See the field doc on `register_invocations`.
    #[inline]
    pub fn register_invocations(&self) -> u64 {
        self.register_invocations
    }

    /// Register an SPU image under `path`. Idempotent for identical
    /// bytes; returns the existing handle.
    ///
    /// # Panics
    /// - `path` is already registered with different `elf_bytes`.
    /// - The monotonic handle counter wraps past `u32::MAX`.
    pub fn register(&mut self, path: &[u8], elf_bytes: Vec<u8>) -> SpuImageHandle {
        self.register_invocations = self.register_invocations.wrapping_add(1);
        debug_assert!(
            path.starts_with(b"/"),
            "ContentStore::register: path {:?} is not absolute",
            String::from_utf8_lossy(path),
        );
        debug_assert!(
            !path.windows(2).any(|w| w == b"//"),
            "ContentStore::register: path {:?} contains '//'",
            String::from_utf8_lossy(path),
        );
        if let Some(existing) = self.by_path.get(path) {
            assert_eq!(
                existing.elf_bytes,
                elf_bytes,
                "ContentStore::register: path {:?} already registered with \
                 {} bytes; cannot re-register with {} bytes",
                String::from_utf8_lossy(path),
                existing.elf_bytes.len(),
                elf_bytes.len(),
            );
            return existing.handle;
        }
        let raw = self.next_handle;
        // next_handle seeds at 1 and the checked_add below panics on
        // wrap, so raw == 0 is reachable only via `seeded_at(0)`,
        // which exists to test this panic.
        let handle =
            SpuImageHandle::new(raw).expect("ContentStore::register: next_handle reached 0");
        self.next_handle = raw
            .checked_add(1)
            .expect("ContentStore handle counter exhausted (u32::MAX images)");
        let record = SpuImageRecord { handle, elf_bytes };
        let prev_path = self.by_path.insert(path.to_vec(), record);
        debug_assert!(
            prev_path.is_none(),
            "by_path had an entry for a path the duplicate-check missed",
        );
        let prev_handle = self.by_handle.insert(handle, path.to_vec());
        debug_assert!(
            prev_handle.is_none(),
            "by_handle collision on freshly-allocated handle",
        );
        handle
    }

    /// Look up an image by path.
    pub fn lookup_by_path(&self, path: &[u8]) -> Option<&SpuImageRecord> {
        self.by_path.get(path)
    }

    /// Look up an image by handle.
    pub fn lookup_by_handle(&self, handle: SpuImageHandle) -> Option<&SpuImageRecord> {
        let path = self.by_handle.get(&handle)?;
        let record = self.by_path.get(path);
        debug_assert!(
            record.is_some(),
            "desync: by_handle has {handle:?} but by_path does not",
        );
        record
    }

    /// Number of registered images.
    pub fn len(&self) -> usize {
        debug_assert_eq!(self.by_path.len(), self.by_handle.len());
        self.by_path.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        debug_assert_eq!(self.by_path.is_empty(), self.by_handle.is_empty());
        self.by_path.is_empty()
    }

    /// Length-prefixed FNV-1a over `(path, handle, elf_bytes)` in
    /// path order; prefixes prevent boundary collisions between
    /// adjacent fields.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (path, record) in &self.by_path {
            hasher.write(&(path.len() as u64).to_le_bytes());
            hasher.write(path);
            hasher.write(&record.handle.raw().to_le_bytes());
            hasher.write(&(record.elf_bytes.len() as u64).to_le_bytes());
            hasher.write(&record.elf_bytes);
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/image_tests.rs"]
mod tests;
