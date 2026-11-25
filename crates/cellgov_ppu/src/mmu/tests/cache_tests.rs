//! The translation cache keeps each side's translations apart and finds
//! the one that holds an address.

use super::*;
use crate::supervisor::SupervisorState;

fn block(ea: u64, ra: u64, len: u64) -> Translation {
    Translation {
        ea,
        ra,
        len,
        writable: true,
        inhibited: false,
        guarded: false,
        changed: false,
    }
}

#[test]
fn a_lookup_finds_the_translation_holding_the_address_and_no_other() {
    let mut cache = TranslationCache::default();
    cache.insert(Access::Load, block(0x1000, 0x9000, 0x1000));
    cache.insert(Access::Store, block(0x4000, 0x2000, 0x1000));
    assert_eq!(
        cache.lookup(Access::Load, 0x1FFF).map(|t| t.ra),
        Some(0x9000)
    );
    assert_eq!(
        cache.lookup(Access::Store, 0x1000).map(|t| t.ra),
        Some(0x9000)
    );
    assert_eq!(
        cache.lookup(Access::Load, 0x4800).map(|t| t.ra),
        Some(0x2000)
    );
    assert!(
        cache.lookup(Access::Load, 0x2000).is_none(),
        "one past the end"
    );
    assert!(
        cache.lookup(Access::Load, 0x0FFF).is_none(),
        "one before the start"
    );
}

#[test]
fn fetch_translations_sit_apart_from_data_translations() {
    let mut cache = TranslationCache::default();
    cache.insert(Access::Fetch, block(0x1000, 0x9000, 0x1000));
    assert!(cache.lookup(Access::Load, 0x1000).is_none());
    assert_eq!(cache.data().count(), 0);
    cache.insert(Access::Load, block(0x1000, 0x5000, 0x1000));
    assert_eq!(
        cache.lookup(Access::Fetch, 0x1000).map(|t| t.ra),
        Some(0x9000)
    );
    assert_eq!(
        cache.lookup(Access::Load, 0x1000).map(|t| t.ra),
        Some(0x5000)
    );
}

#[test]
fn aliases_names_another_data_block_over_the_same_real_bytes() {
    let mut cache = TranslationCache::default();
    cache.insert(Access::Load, block(0x8000_0000_0000_0000, 0, 0x1000_0000));
    assert!(
        !cache.aliases(0x8000_0000_0000_0000, 0x10, 4),
        "the block itself"
    );
    assert!(cache.aliases(0x10, 0x10, 4), "a second EA over its bytes");
    assert!(!cache.aliases(0x10, 0x1000_0000, 4), "past its real end");
    cache.insert(Access::Load, block(0x9000_0000, 0x2000_0000, 0x1000));
    assert!(
        !cache.aliases(0x10, 0x1FFF_FFFC, 4),
        "a range that ends where a block starts"
    );
    assert!(
        cache.aliases(0x10, 0x1FFF_FFFD, 4),
        "one byte into the block"
    );
}

#[test]
fn follow_keeps_the_cache_under_one_context_and_empties_it_under_another() {
    let sup = SupervisorState::new(0x10);
    let mut cache = TranslationCache::default();
    assert!(!cache.made_under(&sup), "an empty cache follows no context");
    cache.follow(&sup);
    cache.insert(Access::Load, block(0x1000, 0x9000, 0x1000));
    cache.follow(&sup);
    assert!(cache.made_under(&sup));
    assert!(!cache.is_empty(), "the same context keeps the translations");

    type Move = Box<dyn Fn(&mut SupervisorState)>;
    let moves: [Move; 4] = [
        Box::new(|s| s.msr ^= 0x20),
        Box::new(|s| s.slb[63].esid_v ^= 1 << 27),
        Box::new(|s| s.slb[0].vsid_flags ^= 1 << 12),
        Box::new(|s| s.partition = Partition::new(0, 1 << 28, 0, s.partition.lb()).unwrap()),
    ];
    for (k, change) in moves.iter().enumerate() {
        let mut moved = sup.clone();
        change(&mut moved);
        let mut c = cache.clone();
        assert!(!c.made_under(&moved), "move {k}");
        c.follow(&moved);
        assert!(c.is_empty(), "move {k} empties the cache");
        assert!(c.made_under(&moved), "move {k}");
    }
    // A register no translation reads leaves the cache alone.
    let mut other = sup.clone();
    other.srr0 = 0x4000;
    other.sprg[3] = 7;
    assert!(cache.made_under(&other));
}

#[test]
fn clear_empties_both_sides() {
    let mut cache = TranslationCache::default();
    assert!(cache.is_empty());
    cache.insert(Access::Fetch, block(0x1000, 0x9000, 0x1000));
    cache.insert(Access::Load, block(0x1000, 0x9000, 0x1000));
    assert!(!cache.is_empty());
    cache.clear();
    assert!(cache.is_empty());
    assert!(cache.lookup(Access::Fetch, 0x1000).is_none());
}
