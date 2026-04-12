//! SPU image registry -- path-keyed content store with deterministic
//! handle allocation.
//!
//! The test harness pre-registers SPU ELF images by path. When the PPU
//! calls `sys_spu_image_open`, the host looks up the path in this store
//! and returns a `SpuImageHandle`. The handle is a monotonic u32 token
//! with no pointer semantics.

use crate::dispatch::SpuImageHandle;
use std::collections::BTreeMap;

/// A registered SPU image.
#[derive(Debug, Clone)]
pub struct SpuImageRecord {
    /// Handle allocated at registration time.
    pub handle: SpuImageHandle,
    /// Raw ELF bytes (the full SPU ELF, not just the loadable segments).
    pub elf_bytes: Vec<u8>,
}

/// Path-keyed content store for SPU images.
///
/// Maps path bytes (as they appear in guest memory, typically
/// NUL-terminated ASCII) to `SpuImageRecord`. Handles are allocated
/// by a monotonic counter starting at 1.
///
/// No host filesystem access. No path normalization beyond
/// byte-equality lookup.
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

    /// Register an SPU image under `path`. Returns the allocated
    /// handle. If the path is already registered, returns the
    /// existing handle without modifying the record.
    pub fn register(&mut self, path: &[u8], elf_bytes: Vec<u8>) -> SpuImageHandle {
        if let Some(existing) = self.by_path.get(path) {
            return existing.handle;
        }
        let handle = SpuImageHandle::new(self.next_handle);
        self.next_handle += 1;
        let record = SpuImageRecord { handle, elf_bytes };
        self.by_path.insert(path.to_vec(), record);
        self.by_handle.insert(handle, path.to_vec());
        handle
    }

    /// Look up an image by path. Returns `None` if the path is not
    /// registered.
    pub fn lookup_by_path(&self, path: &[u8]) -> Option<&SpuImageRecord> {
        self.by_path.get(path)
    }

    /// Look up an image by handle. Returns `None` if the handle is
    /// not registered.
    pub fn lookup_by_handle(&self, handle: SpuImageHandle) -> Option<&SpuImageRecord> {
        let path = self.by_handle.get(&handle)?;
        self.by_path.get(path)
    }

    /// Number of registered images.
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// FNV-1a hash of the store contents for determinism checking.
    ///
    /// Covers every `(path, handle)` pair in path order. An empty
    /// store returns the FNV offset basis.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (path, record) in &self.by_path {
            hasher.write(path);
            hasher.write(&record.handle.raw().to_le_bytes());
        }
        hasher.finish()
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
    fn register_same_path_returns_existing_handle() {
        let mut s = ContentStore::new();
        let h1 = s.register(b"/app_home/spu.elf", vec![1, 2]);
        let h2 = s.register(b"/app_home/spu.elf", vec![9, 9]);
        assert_eq!(h1, h2);
        assert_eq!(s.len(), 1);
        assert_eq!(
            s.lookup_by_path(b"/app_home/spu.elf").unwrap().elf_bytes,
            vec![1, 2]
        );
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
        assert!(s.lookup_by_handle(SpuImageHandle::new(99)).is_none());
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
    fn state_hash_differs_on_content() {
        let mut a = ContentStore::new();
        let mut b = ContentStore::new();
        a.register(b"/a.elf", vec![1]);
        b.register(b"/b.elf", vec![1]);
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
}
