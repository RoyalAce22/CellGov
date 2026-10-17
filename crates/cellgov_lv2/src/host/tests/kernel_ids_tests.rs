//! The kernel-id slot table: lowest free slot, a generation per reuse,
//! a shared slot only once every slot is live, and the process limit.

use std::collections::BTreeSet;

use cellgov_ps3_abi::lv2::kernel_id::{compose, MAX_LIVE_IDS, SLOT_COUNT};
use cellgov_ps3_abi::lv2::process::{SYS_MUTEX_OBJECT, SYS_RWLOCK_OBJECT, SYS_SEMAPHORE_OBJECT};

use super::*;

#[test]
fn creates_take_consecutive_slots_at_generation_zero_across_classes() {
    let mut t = KernelIdTable::default();
    assert_eq!(
        t.alloc(SYS_MUTEX_OBJECT),
        Some(compose(SYS_MUTEX_OBJECT, 0, 0))
    );
    assert_eq!(
        t.alloc(SYS_SEMAPHORE_OBJECT),
        Some(compose(SYS_SEMAPHORE_OBJECT, 0, 1))
    );
    assert_eq!(t.alloc(0x30), Some(compose(0x30, 0, 2)));
}

#[test]
fn a_released_slot_is_the_next_create_one_generation_up() {
    let mut t = KernelIdTable::default();
    let ids: Vec<u32> = (0..3).map(|_| t.alloc(0x95).unwrap()).collect();
    for &id in &ids {
        assert!(t.release(id));
    }
    assert_eq!(
        t.alloc(SYS_RWLOCK_OBJECT),
        Some(compose(SYS_RWLOCK_OBJECT, 1, 0))
    );
    assert_eq!(
        t.alloc(SYS_RWLOCK_OBJECT),
        Some(compose(SYS_RWLOCK_OBJECT, 1, 1))
    );
}

#[test]
fn the_lowest_free_slot_wins_over_a_higher_released_one() {
    let mut t = KernelIdTable::default();
    let a = t.alloc(0x85).unwrap();
    let b = t.alloc(0x85).unwrap();
    assert!(t.release(b));
    assert!(t.release(a));
    assert_eq!(t.alloc(0x85), Some(compose(0x85, 1, 0)));
}

#[test]
fn reusing_one_slot_walks_its_generations_and_wraps() {
    let mut t = KernelIdTable::default();
    for generation in 0..300u32 {
        let id = t.alloc(0x20).unwrap();
        assert_eq!(id, compose(0x20, generation, 0), "create {generation}");
        assert!(t.release(id));
    }
}

#[test]
fn a_release_needs_the_live_ids_class_and_generation() {
    let mut t = KernelIdTable::default();
    let id = t.alloc(SYS_MUTEX_OBJECT).unwrap();
    assert!(
        !t.release(compose(SYS_SEMAPHORE_OBJECT, 0, 0)),
        "other class"
    );
    assert!(
        !t.release(compose(SYS_MUTEX_OBJECT, 1, 0)),
        "other generation"
    );
    assert!(t.is_live(id));
    assert!(t.release(id));
    assert!(!t.is_live(id));
    assert!(!t.release(id), "already released");
}

#[test]
fn with_every_slot_live_a_create_shares_the_lowest_slot_at_a_fresh_generation() {
    let mut t = KernelIdTable::default();
    let mut seen = BTreeSet::new();
    for _ in 0..SLOT_COUNT {
        assert!(seen.insert(t.alloc(0x85).unwrap()));
    }
    let shared = t.alloc(0x85).unwrap();
    assert_eq!(shared, compose(0x85, 1, 0));
    assert!(seen.insert(shared));
    assert_eq!(t.alloc(0x85), Some(compose(0x85, 1, 1)));
}

#[test]
fn ids_stay_unique_up_to_the_process_limit_and_the_next_is_refused() {
    let mut t = KernelIdTable::default();
    let mut seen = BTreeSet::new();
    for n in 0..MAX_LIVE_IDS {
        let id = t
            .alloc(0x85)
            .unwrap_or_else(|| panic!("create {n} refused"));
        assert!(seen.insert(id & 0x00FF_FFFF), "create {n} repeated {id:#x}");
    }
    assert_eq!(t.alloc(0x85), None);
    let freed = *seen.iter().next().unwrap() | (0x85 << 24);
    assert!(t.release(freed));
    assert!(t.alloc(0x85).is_some());
}
