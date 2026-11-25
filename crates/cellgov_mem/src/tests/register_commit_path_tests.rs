//! Which commit path a store to a register word may take: the staging
//! drain applies it through the device's rule, and `apply_commit` refuses
//! it.

use super::*;
use crate::{GuestAddr, PageSize, Region, StagedWrite, StagingMemory};

const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);
const WORD: u64 = SPAN.0 + 0x10;

/// The word in the state of its declaration.
const DECLARED: RegisterWord = RegisterWord {
    value: 0x10,
    known: 0xF0,
};

/// A rule that accepts every store and makes the stored word known.
fn take_value(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    words[index] = RegisterWord::known(store.value);
    true
}

fn memory() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem.declare_registers(&[(WORD, DECLARED)], take_value)
        .unwrap();
    mem
}

fn word_range() -> ByteRange {
    ByteRange::new(GuestAddr::new(WORD), 4).unwrap()
}

#[test]
fn apply_commit_refuses_a_register_word_as_unmapped_and_keeps_the_device_state() {
    let mut mem = memory();
    let hash = mem.content_hash();
    let applied = mem.apply_commit(word_range(), &[0, 0, 0, 2]);
    assert!(
        matches!(
            applied,
            Err(MemError::Unmapped(FaultContext { addr, .. })) if addr == WORD
        ),
        "{applied:?}"
    );
    assert_eq!(mem.register_word(WORD), Some(DECLARED));
    assert_eq!(mem.register_bytes(WORD), Some(&[0u8, 0, 0, 0x10][..]));
    assert_eq!(mem.content_hash(), hash);
}

#[test]
fn the_staging_drain_applies_the_same_store_through_the_device_rule() {
    let mut mem = memory();
    let mut staging = StagingMemory::new();
    staging.stage(StagedWrite::new(word_range(), &[0, 0, 0, 2]));
    assert_eq!(staging.drain_into(&mut mem), Ok(1));
    assert_eq!(mem.register_word(WORD), Some(RegisterWord::known(2)));
    assert_eq!(mem.register_bytes(WORD), Some(&[0u8, 0, 0, 2][..]));
}
