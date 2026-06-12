//! SPU image handle and content-store tests -- registration idempotence and lookup by path or handle.

use super::*;

#[test]
fn spu_image_handle_roundtrip() {
    let h = SpuImageHandle::new(42).unwrap();
    assert_eq!(h.raw(), 42);
}

#[test]
fn spu_image_handle_zero_rejected() {
    assert!(SpuImageHandle::new(0).is_none());
}

#[test]
fn spu_image_handle_ordering() {
    assert!(SpuImageHandle::new(1).unwrap() < SpuImageHandle::new(2).unwrap());
}

#[test]
fn new_store_is_empty() {
    let s = ContentStore::new();
    assert!(s.is_empty());
    assert_eq!(s.len(), 0);
}

#[test]
fn register_invocations_counter_increments_per_call() {
    // C-5a audit witness: register_invocations counts every
    // register() call, regardless of whether the registration
    // was novel or matched an existing path. Proves the path-
    // shape debug_assert!s' silence is non-vacuous when > 0.
    let mut s = ContentStore::new();
    assert_eq!(s.register_invocations(), 0);
    s.register(b"/spu_a.elf", vec![1]);
    assert_eq!(s.register_invocations(), 1);
    // Idempotent registration still counts as a register call --
    // the guards still ran on the path-shape check.
    s.register(b"/spu_a.elf", vec![1]);
    assert_eq!(s.register_invocations(), 2);
    s.register(b"/spu_b.elf", vec![2]);
    assert_eq!(s.register_invocations(), 3);
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

impl ContentStore {
    /// Seed `next_handle` directly so tests can reach the two
    /// register-panic sites `new` makes unreachable.
    fn seeded_at(next_handle: u32) -> Self {
        Self {
            by_path: BTreeMap::new(),
            by_handle: BTreeMap::new(),
            next_handle,
            register_invocations: 0,
        }
    }
}
