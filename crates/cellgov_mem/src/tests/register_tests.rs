//! Register words: which loads they answer, which bits of a store are the
//! storing code's own, and how a store commits.

use super::*;
use crate::{GuestAddr, PageSize, Region, StagedWrite, StagingMemory};

/// A 16 KB region at 0 and a 64 KB span at 0x24003810000.
const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);

/// The two words of the test file.
const WORD: u64 = SPAN.0 + 0x10;
const NEXT: u64 = SPAN.0 + 0x14;

const READER: u64 = 0x8000_0000_0024_22E0;

/// The store that writes the reader's loaded word back.
const STORE: u64 = READER + 0x10;

/// The pass-back of `bits` by [`STORE`].
fn back(bits: u32) -> Option<RegisterPassBack> {
    Some(RegisterPassBack { store: STORE, bits })
}

fn memory() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem
}

/// A rule that makes the stored word's own bits the new value of every
/// word.
fn own_as_value(words: &mut [RegisterWord], _: usize, store: RegisterStore) -> bool {
    words.fill(RegisterWord::known(store.own));
    true
}

/// A rule that refuses a store with bit 0 set and keeps the stored word
/// otherwise.
fn refuse_bit_0(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    if store.value & 1 != 0 {
        return false;
    }
    words[index] = RegisterWord::known(store.value);
    true
}

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), 4).unwrap()
}

#[test]
fn a_reader_gets_the_word_only_while_the_bits_it_keeps_are_known() {
    let mut mem = memory();
    let word = RegisterWord {
        value: 0x1000,
        known: 0x1000,
    };
    mem.declare_registers(&[(WORD, word)], refuse_bit_0)
        .unwrap();
    mem.declare_register_reader(WORD, READER, 0x1000, None)
        .unwrap();
    mem.declare_register_reader(WORD, READER + 4, 0x1001, None)
        .unwrap();
    let bytes: &[u8] = &[0, 0, 0x10, 0];
    assert_eq!(mem.register_read(WORD + 2, READER), Some((WORD, bytes)));
    assert_eq!(mem.register_read(WORD, READER + 4), None);
    assert_eq!(mem.register_read(WORD, READER + 8), None);
}

#[test]
fn a_stored_zero_is_the_code_own_unless_a_reader_passes_it_back_unknown() {
    // (passes, kept, stored value, own bits)
    let cases: &[(u32, u32, u32, u32)] = &[
        (!1, 0, 0, 1),
        (!1, 0, 6, 7),
        (0, 0, 0, u32::MAX),
        (!2, 1, 0, 3),
    ];
    assert_eq!(cases.len(), 4);
    for &(passes, kept, value, own) in cases {
        let mut mem = memory();
        mem.declare_registers(&[(WORD, RegisterWord::UNKNOWN)], own_as_value)
            .unwrap();
        mem.declare_register_reader(WORD, READER, kept, back(passes))
            .unwrap();
        mem.apply_commit(range(WORD), &value.to_be_bytes()).unwrap();
        assert_eq!(
            mem.register_word(WORD),
            Some(RegisterWord::known(own)),
            "{passes:x} {kept:x} {value:x}"
        );
    }
}

#[test]
fn a_known_bit_of_a_store_is_the_code_own_even_when_a_reader_passes_it() {
    let mut mem = memory();
    let word = RegisterWord {
        value: 0,
        known: 0xF,
    };
    mem.declare_registers(&[(WORD, word)], own_as_value)
        .unwrap();
    mem.declare_register_reader(WORD, READER, 0, back(u32::MAX))
        .unwrap();
    mem.apply_commit(range(WORD), &[0; 4]).unwrap();
    assert_eq!(mem.register_word(WORD), Some(RegisterWord::known(0xF)));
}

#[test]
fn a_stored_zero_passes_back_only_from_the_store_its_reader_names() {
    let mut mem = memory();
    mem.declare_registers(&[(WORD, RegisterWord::UNKNOWN)], refuse_unknown_own)
        .unwrap();
    mem.declare_register_reader(WORD, READER, 0, back(!1))
        .unwrap();
    // (storing instruction, stored value, accepted)
    let cases: &[(u64, u32, bool)] = &[
        (STORE, 0, true),
        (STORE + 4, 0, false),
        (READER, 0, false),
        (STORE + 4, u32::MAX - 1, true),
    ];
    assert_eq!(cases.len(), 4);
    for &(store, value, accepted) in cases {
        assert_eq!(
            mem.register_store_accepted(WORD, value, store),
            accepted,
            "{store:x} {value:x}"
        );
    }
}

/// A rule that refuses a store that owns a 0 in a bit other than bit 0,
/// and keeps the words otherwise.
fn refuse_unknown_own(_: &mut [RegisterWord], _: usize, store: RegisterStore) -> bool {
    store.own & !store.value & !1 == 0
}

#[test]
fn a_refused_store_fails_validation_and_changes_no_word() {
    let mut mem = memory();
    mem.declare_registers(&[(WORD, RegisterWord::UNKNOWN)], refuse_bit_0)
        .unwrap();
    let hash = mem.content_hash();
    let refused = Err(MemError::RegisterStoreRefused { real: WORD });
    assert!(!mem.register_store_accepted(WORD, 1, STORE));
    assert!(mem.register_store_accepted(WORD, 2, STORE));
    assert_eq!(mem.validate_commit(range(WORD), &[0, 0, 0, 1]), refused);
    assert_eq!(mem.apply_commit(range(WORD), &[0, 0, 0, 1]), refused);
    assert_eq!(mem.register_word(WORD), Some(RegisterWord::UNKNOWN));
    assert_eq!(mem.content_hash(), hash);
}

#[test]
fn an_accepted_store_commits_through_staging_and_moves_the_content_hash() {
    let mut mem = memory();
    mem.declare_registers(
        &[(WORD, RegisterWord::UNKNOWN), (NEXT, RegisterWord::UNKNOWN)],
        refuse_bit_0,
    )
    .unwrap();
    let hash = mem.content_hash();
    let mut staging = StagingMemory::new();
    staging.stage(StagedWrite::new(range(NEXT), &[0, 0, 0, 2]));
    assert_eq!(staging.drain_into(&mut mem), Ok(1));
    assert_eq!(mem.register_word(NEXT), Some(RegisterWord::known(2)));
    assert_eq!(mem.register_bytes(NEXT), Some(&[0u8, 0, 0, 2][..]));
    assert_ne!(mem.content_hash(), hash);
    assert_eq!(mem.content_hash(), mem.content_hash_from_scratch());
}

#[test]
fn a_batch_with_two_register_stores_commits_neither() {
    let mut mem = memory();
    mem.declare_registers(
        &[(WORD, RegisterWord::UNKNOWN), (NEXT, RegisterWord::UNKNOWN)],
        refuse_bit_0,
    )
    .unwrap();
    let mut staging = StagingMemory::new();
    staging.stage(StagedWrite::new(range(WORD), &[0, 0, 0, 2]));
    staging.stage(StagedWrite::new(range(NEXT), &[0, 0, 0, 4]));
    assert_eq!(
        staging.drain_into(&mut mem),
        Err(MemError::RegisterStoreRefused { real: NEXT })
    );
    staging.clear();
    assert_eq!(mem.register_word(WORD), Some(RegisterWord::UNKNOWN));
    assert_eq!(mem.register_word(NEXT), Some(RegisterWord::UNKNOWN));
}

#[test]
fn a_register_store_and_an_ordinary_write_in_one_batch_commit_together_or_not_at_all() {
    // (stored value, drain result, region word after, register word after)
    type Case = (u32, Result<usize, MemError>, [u8; 4], RegisterWord);
    let cases: &[Case] = &[
        (2, Ok(2), [0xAA; 4], RegisterWord::known(2)),
        (
            1,
            Err(MemError::RegisterStoreRefused { real: WORD }),
            [0; 4],
            RegisterWord::UNKNOWN,
        ),
    ];
    assert_eq!(cases.len(), 2);
    for &(value, drained, region, word) in cases {
        let mut mem = memory();
        mem.declare_registers(&[(WORD, RegisterWord::UNKNOWN)], refuse_bit_0)
            .unwrap();
        let mut staging = StagingMemory::new();
        staging.stage(StagedWrite::new(range(0x100), &[0xAA; 4]));
        staging.stage(StagedWrite::new(range(WORD), &value.to_be_bytes()));
        assert_eq!(staging.drain_into(&mut mem), drained, "{value}");
        staging.clear();
        assert_eq!(
            mem.read(range(0x100)).as_deref(),
            Some(&region[..]),
            "{value}"
        );
        assert_eq!(mem.register_word(WORD), Some(word), "{value}");
    }
}

#[test]
fn a_write_that_is_not_one_whole_register_word_fails_alike_at_validation_and_commit() {
    // (start, length): unaligned, partial, one byte, two words, a word
    // and the bytes before it.
    let cases: &[(u64, u64)] = &[
        (WORD + 2, 4),
        (WORD, 2),
        (WORD + 1, 1),
        (WORD, 8),
        (WORD - 4, 8),
    ];
    assert_eq!(cases.len(), 5);
    for &(start, length) in cases {
        let mut mem = memory();
        mem.declare_registers(
            &[(WORD, RegisterWord::UNKNOWN), (NEXT, RegisterWord::UNKNOWN)],
            own_as_value,
        )
        .unwrap();
        let write = ByteRange::new(GuestAddr::new(start), length).unwrap();
        let bytes = vec![0xFF; length as usize];
        assert!(!mem.is_register(start, length), "{start:x}+{length}");
        let validated = mem.validate_commit(write, &bytes);
        assert!(
            matches!(
                validated,
                Err(MemError::Unmapped(FaultContext { addr, .. })) if addr == start
            ),
            "{start:x}+{length}: {validated:?}"
        );
        assert_eq!(
            mem.apply_commit(write, &bytes),
            validated,
            "{start:x}+{length}"
        );
        assert_eq!(mem.register_word(WORD), Some(RegisterWord::UNKNOWN));
        assert_eq!(mem.register_word(NEXT), Some(RegisterWord::UNKNOWN));
    }
}

#[test]
fn a_reset_for_reuse_returns_each_word_to_its_declared_state() {
    let mut mem = memory();
    let declared = RegisterWord {
        value: 0x10,
        known: 0xF0,
    };
    mem.declare_registers(&[(WORD, declared)], refuse_bit_0)
        .unwrap();
    mem.apply_commit(range(WORD), &[0, 0, 0, 2]).unwrap();
    assert_eq!(mem.register_word(WORD), Some(RegisterWord::known(2)));
    mem.reset_for_reuse();
    assert_eq!(mem.register_word(WORD), Some(declared));
    assert_eq!(mem.register_bytes(WORD), Some(&[0u8, 0, 0, 0x10][..]));
}

#[test]
fn the_memory_refuses_each_declaration_that_names_no_word_it_can_hold() {
    let mut mem = memory();
    let refused = |real| Err(MemError::RegisterRefused { real });
    let unknown = RegisterWord::UNKNOWN;
    let rule: RegisterRule = refuse_bit_0;
    assert_eq!(
        mem.declare_registers(&[(WORD + 2, unknown)], rule),
        refused(WORD + 2)
    );
    assert_eq!(
        mem.declare_registers(&[(0x100, unknown)], rule),
        refused(0x100)
    );
    let end = SPAN.0 + SPAN.1;
    assert_eq!(mem.declare_registers(&[(end, unknown)], rule), refused(end));
    assert_eq!(
        mem.declare_registers(&[(SPAN.0, unknown), (end - 4, unknown)], rule),
        Ok(())
    );
    let loose = RegisterWord { value: 2, known: 1 };
    assert_eq!(mem.declare_registers(&[(WORD, loose)], rule), refused(WORD));
    assert_eq!(
        mem.declare_registers(&[(WORD, unknown), (WORD, unknown)], rule),
        refused(WORD)
    );
    assert_eq!(mem.register_word(WORD), None);
    mem.declare_registers(&[(WORD, unknown)], rule).unwrap();
    assert_eq!(
        mem.declare_registers(&[(WORD, unknown)], rule),
        refused(WORD)
    );
    assert_eq!(
        mem.declare_register_reader(NEXT, READER, 0, None),
        refused(NEXT)
    );
    mem.declare_register_reader(WORD, READER, 0, None).unwrap();
    assert_eq!(
        mem.declare_register_reader(WORD, READER, 0, None),
        refused(WORD)
    );
}
