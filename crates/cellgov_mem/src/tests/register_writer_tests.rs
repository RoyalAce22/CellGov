//! Register writers: a word with a declared writer takes a store only
//! from that instruction.

use super::*;
use crate::{PageSize, Region};

const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);

/// The two words of the test register file.
const WORD: u64 = SPAN.0 + 0x94;
const NEXT: u64 = SPAN.0 + 0x9C;

const WRITER: u64 = 0x8000_0000_0023_F964;

fn declared() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem.declare_registers(
        &[(WORD, RegisterWord::UNKNOWN), (NEXT, RegisterWord::UNKNOWN)],
        keep_value,
    )
    .unwrap();
    mem
}

/// A rule that keeps every stored word.
fn keep_value(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    words[index] = RegisterWord::known(store.value);
    true
}

#[test]
fn a_word_with_a_writer_takes_a_store_only_from_that_writer() {
    let mut mem = declared();
    mem.declare_register_writer(WORD, WRITER).unwrap();
    // (word, store instruction, accepted)
    let cases: &[(u64, u64, bool)] = &[
        (WORD, WRITER, true),
        (WORD, WRITER + 4, false),
        (WORD, WRITER - 4, false),
        (NEXT, WRITER + 4, true),
    ];
    assert_eq!(cases.len(), 4);
    for &(real, store, accepted) in cases {
        assert_eq!(
            mem.register_store_accepted(real, 0x0100_0020, store),
            accepted,
            "{real:x} {store:x}"
        );
    }
}

#[test]
fn a_writer_of_one_word_is_no_writer_of_another() {
    let mut mem = declared();
    // Out of address order, so a lookup over an unsorted list misses.
    mem.declare_register_writer(NEXT, WRITER + 8).unwrap();
    mem.declare_register_writer(WORD, WRITER + 4).unwrap();
    mem.declare_register_writer(WORD, WRITER).unwrap();
    // (word, store instruction, accepted)
    let cases: &[(u64, u64, bool)] = &[
        (WORD, WRITER, true),
        (WORD, WRITER + 4, true),
        (WORD, WRITER + 8, false),
        (NEXT, WRITER + 8, true),
        (NEXT, WRITER, false),
    ];
    assert_eq!(cases.len(), 5);
    for &(real, store, accepted) in cases {
        assert_eq!(
            mem.register_store_accepted(real, 0x0100_0020, store),
            accepted,
            "{real:x} {store:x}"
        );
    }
}

#[test]
fn a_writer_declaration_needs_a_word_and_a_new_store() {
    let mut mem = declared();
    mem.declare_register_writer(WORD, WRITER).unwrap();
    mem.declare_register_writer(WORD, WRITER + 4).unwrap();
    // (word, store instruction, refusal)
    let cases: &[(u64, u64, MemError)] = &[
        (
            WORD,
            WRITER,
            MemError::RegisterWriterRepeated {
                real: WORD,
                store: WRITER,
            },
        ),
        (
            WORD + 4,
            WRITER,
            MemError::NoRegisterWord { real: WORD + 4 },
        ),
        (SPAN.0, WRITER, MemError::NoRegisterWord { real: SPAN.0 }),
    ];
    assert_eq!(cases.len(), 3);
    for &(real, store, refusal) in cases {
        assert_eq!(
            mem.declare_register_writer(real, store),
            Err(refusal),
            "{real:x} {store:x}"
        );
    }
    assert!(mem.register_store_accepted(WORD, 1, WRITER + 4));
}
