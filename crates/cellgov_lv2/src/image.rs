//! SPU image registry: path-keyed ELF records and user-type segment
//! images, sharing one monotonic non-zero handle counter.

use cellgov_mem::lanes::{self, object_digest, source, LaneMap, LaneValue, ObjectLanes};
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

/// One resolved local-store segment of a user image: `bytes` land at
/// `ls_start`. FILL segments are expanded to bytes when the record is
/// built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsSegment {
    /// Local-store destination.
    pub ls_start: u32,
    /// Bytes to place there.
    pub bytes: Vec<u8>,
}

/// A user-type SPU image: the caller laid out its segments itself
/// (`sys_spu_image` with `type == SYS_SPU_IMAGE_TYPE_USER`), so there
/// is no ELF and no path, only the entry and the resolved segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSpuImage {
    /// Allocated at registration time; non-zero, disjoint from the
    /// path-keyed handles.
    pub handle: SpuImageHandle,
    /// Local-store entry point.
    pub entry: u32,
    /// Segments in table order.
    pub segments: Vec<LsSegment>,
}

impl LaneValue for SpuImageRecord {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.handle.raw()));
        lanes.bytes(2, &[], &self.elf_bytes);
    }
}

impl LaneValue for UserSpuImage {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.entry));
        lanes.lane(2, 0, self.segments.len() as u64);
        for (slot, segment) in self.segments.iter().enumerate() {
            lanes.lane(3, slot as u64, u64::from(segment.ls_start));
            lanes.bytes(4, &[slot as u64], &segment.bytes);
        }
    }
}

/// Store for registered SPU images.
///
/// # Invariants
/// - `by_path` and `by_handle` agree: every handle in either map
///   resolves through both. Debug-asserts in `register`,
///   `lookup_by_handle`, `len`, and `is_empty` guard the pairing.
/// - No host filesystem access; lookup is byte-exact, so `/a.elf`,
///   `/a.elf/`, and `//a.elf` are three distinct entries.
/// - `user_images` handles come from the same counter as path-keyed
///   handles, so a handle resolves in at most one of the two maps.
#[derive(Debug, Clone)]
pub struct ContentStore {
    /// Keyed by path; each image's lane object is the path's digest.
    by_path: LaneMap<Vec<u8>, SpuImageRecord>,
    /// The inverse of `by_path`, derived from it and not hashed.
    by_handle: BTreeMap<SpuImageHandle, Vec<u8>>,
    user_images: LaneMap<SpuImageHandle, UserSpuImage>,
    next_handle: u32,
    /// Cumulative count of [`Self::register`] invocations.
    /// Non-vacuity witness: the path-shape `debug_assert!`s in `register`
    /// are conditional on register being called; this counter makes their silence
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
            by_path: LaneMap::new_keyed_by_ref(source::IMAGE_PATH, |path: &Vec<u8>| {
                object_digest(path)
            }),
            by_handle: BTreeMap::new(),
            user_images: LaneMap::new(source::IMAGE_USER, |handle: SpuImageHandle| {
                u64::from(handle.raw())
            }),
            next_handle: 1,
            register_invocations: 0,
        }
    }

    /// Register a user-type image and return its handle. Every call
    /// mints a new handle: user images carry no key to dedupe on.
    ///
    /// # Panics
    /// The monotonic handle counter wraps past `u32::MAX`.
    pub fn register_user_image(&mut self, entry: u32, segments: Vec<LsSegment>) -> SpuImageHandle {
        let raw = self.next_handle;
        let handle = SpuImageHandle::new(raw)
            .expect("ContentStore::register_user_image: next_handle reached 0");
        self.next_handle = raw
            .checked_add(1)
            .expect("ContentStore handle counter exhausted (u32::MAX images)");
        let prev = self.user_images.insert(
            handle,
            UserSpuImage {
                handle,
                entry,
                segments,
            },
        );
        debug_assert!(
            prev.is_none(),
            "user_images collision on freshly-allocated handle"
        );
        handle
    }

    /// Look up a user-type image by handle.
    pub fn lookup_user_image(&self, handle: SpuImageHandle) -> Option<&UserSpuImage> {
        self.user_images.get(handle)
    }

    /// Drop a user-type image registered by [`Self::register_user_image`]
    /// whose thread initialize was refused after registration. The
    /// handle is not reused.
    pub fn withdraw_user_image(&mut self, handle: SpuImageHandle) -> Option<UserSpuImage> {
        self.user_images.remove(handle)
    }

    /// Number of registered user-type images.
    pub fn user_image_count(&self) -> usize {
        self.user_images.len()
    }

    /// Non-vacuity witness: cumulative count of `register` calls.
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
        if let Some(existing) = self.by_path.get_by(path) {
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
        self.by_path.get_by(path)
    }

    /// Look up an image by handle.
    pub fn lookup_by_handle(&self, handle: SpuImageHandle) -> Option<&SpuImageRecord> {
        let path = self.by_handle.get(&handle)?;
        let record = self.by_path.get_by(path.as_slice());
        debug_assert!(
            record.is_some(),
            "desync: by_handle has {handle:?} but by_path does not",
        );
        record
    }

    /// Number of path-keyed images; user images are counted by
    /// [`Self::user_image_count`].
    pub fn len(&self) -> usize {
        debug_assert_eq!(self.by_path.len(), self.by_handle.len());
        self.by_path.len()
    }

    /// Whether the path-keyed map is empty; user images are not counted.
    pub fn is_empty(&self) -> bool {
        debug_assert_eq!(self.by_path.is_empty(), self.by_handle.is_empty());
        self.by_path.is_empty()
    }

    /// The store's partial of the sync-state sum: the path-keyed images,
    /// the user images and the handle counter.
    pub fn sync_partial(&self) -> u128 {
        self.by_path
            .partial()
            .wrapping_add(self.user_images.partial())
            .wrapping_add(self.next_handle_term())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.by_path
            .partial_from_scratch()
            .wrapping_add(self.user_images.partial_from_scratch())
            .wrapping_add(self.next_handle_term())
    }

    fn next_handle_term(&self) -> u128 {
        lanes::value_term(source::IMAGE_NEXT_HANDLE, 0, &self.next_handle)
    }
}

#[cfg(test)]
#[path = "tests/image_tests.rs"]
mod tests;
