//! Path-keyed SPU image registry returning monotonic non-zero handles.

use crate::dispatch::SpuImageHandle;
use std::collections::BTreeMap;

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
        }
    }

    /// Register an SPU image under `path`. Idempotent for identical
    /// bytes; returns the existing handle.
    ///
    /// # Panics
    /// - `path` is already registered with different `elf_bytes`.
    /// - The monotonic handle counter wraps past `u32::MAX`.
    pub fn register(&mut self, path: &[u8], elf_bytes: Vec<u8>) -> SpuImageHandle {
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

    /// Seed `next_handle` directly so tests can reach the two
    /// register-panic sites `new` makes unreachable.
    #[cfg(test)]
    fn seeded_at(next_handle: u32) -> Self {
        Self {
            by_path: BTreeMap::new(),
            by_handle: BTreeMap::new(),
            next_handle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_store_is_empty() {
        let s = ContentStore::new();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn register_allocates_handles_starting_at_1() {
        let mut s = ContentStore::new();
        let h1 = s.register(b"/app_home/spu_a.elf", vec![1, 2, 3]);
        let h2 = s.register(b"/app_home/spu_b.elf", vec![4, 5, 6]);
        assert_eq!(h1.raw(), 1);
        assert_eq!(h2.raw(), 2);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn register_same_path_and_bytes_is_idempotent() {
        let mut s = ContentStore::new();
        let h1 = s.register(b"/app_home/spu.elf", vec![1, 2]);
        let h2 = s.register(b"/app_home/spu.elf", vec![1, 2]);
        assert_eq!(h1, h2);
        assert_eq!(s.len(), 1);
        assert_eq!(
            s.lookup_by_path(b"/app_home/spu.elf").unwrap().elf_bytes,
            vec![1, 2]
        );
    }

    #[test]
    #[should_panic(expected = "cannot re-register")]
    fn register_same_path_different_bytes_panics() {
        let mut s = ContentStore::new();
        s.register(b"/app_home/spu.elf", vec![1, 2]);
        s.register(b"/app_home/spu.elf", vec![9, 9]);
    }

    #[test]
    fn lookup_by_path_returns_record() {
        let mut s = ContentStore::new();
        s.register(b"/app_home/spu.elf", vec![0xAA, 0xBB]);
        let record = s.lookup_by_path(b"/app_home/spu.elf").unwrap();
        assert_eq!(record.handle.raw(), 1);
        assert_eq!(record.elf_bytes, vec![0xAA, 0xBB]);
    }

    #[test]
    fn lookup_by_path_unknown_returns_none() {
        let s = ContentStore::new();
        assert!(s.lookup_by_path(b"/nonexistent").is_none());
    }

    #[test]
    fn lookup_by_handle_returns_record() {
        let mut s = ContentStore::new();
        let h = s.register(b"/app_home/spu.elf", vec![1]);
        let record = s.lookup_by_handle(h).unwrap();
        assert_eq!(record.elf_bytes, vec![1]);
    }

    #[test]
    fn lookup_by_handle_unknown_returns_none() {
        let s = ContentStore::new();
        assert!(s
            .lookup_by_handle(SpuImageHandle::new(99).unwrap())
            .is_none());
    }

    #[test]
    fn state_hash_is_deterministic() {
        let mut a = ContentStore::new();
        let mut b = ContentStore::new();
        a.register(b"/a.elf", vec![1]);
        a.register(b"/b.elf", vec![2]);
        b.register(b"/a.elf", vec![1]);
        b.register(b"/b.elf", vec![2]);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_on_path() {
        let mut a = ContentStore::new();
        let mut b = ContentStore::new();
        a.register(b"/a.elf", vec![1]);
        b.register(b"/b.elf", vec![1]);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_differs_on_elf_bytes() {
        let mut a = ContentStore::new();
        let mut b = ContentStore::new();
        a.register(b"/spu.elf", vec![0xAA, 0xBB, 0xCC]);
        b.register(b"/spu.elf", vec![0xAA, 0xBB, 0xCD]);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_length_prefix_prevents_boundary_collision() {
        // Without length prefixes, ("/a", "bc") and ("/ab", "c") collide.
        let mut a = ContentStore::new();
        let mut b = ContentStore::new();
        a.register(b"/a", vec![b'b', b'c']);
        b.register(b"/ab", vec![b'c']);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_empty_vs_populated_differ() {
        let empty = ContentStore::new();
        let mut populated = ContentStore::new();
        populated.register(b"/a.elf", vec![]);
        assert_ne!(empty.state_hash(), populated.state_hash());
    }

    #[test]
    fn handle_zero_is_never_allocated() {
        let mut s = ContentStore::new();
        for i in 0..10 {
            s.register(format!("/img_{i}").as_bytes(), vec![]);
        }
        for i in 0..10 {
            let record = s.lookup_by_path(format!("/img_{i}").as_bytes()).unwrap();
            assert_ne!(record.handle.raw(), 0);
        }
    }

    #[test]
    #[should_panic(expected = "ContentStore handle counter exhausted")]
    fn register_panics_when_counter_exhausted() {
        let mut s = ContentStore::seeded_at(u32::MAX);
        s.register(b"/first.elf", vec![]);
        s.register(b"/second.elf", vec![]);
    }

    #[test]
    #[should_panic(expected = "next_handle reached 0")]
    fn register_panics_when_next_handle_is_zero() {
        let mut s = ContentStore::seeded_at(0);
        s.register(b"/first.elf", vec![]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "is not absolute")]
    fn register_debug_asserts_path_starts_with_slash() {
        let mut s = ContentStore::new();
        s.register(b"relative/spu.elf", vec![]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "contains '//'")]
    fn register_debug_asserts_no_double_slash() {
        let mut s = ContentStore::new();
        s.register(b"/app_home//spu.elf", vec![]);
    }
}
