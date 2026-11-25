//! Declaration overlaps: the memory refuses a word that is both a sourced
//! word and a register word, and accepts the two kinds at adjacent words.

use super::*;
use crate::{PageSize, Region};

const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);

const WORD: u64 = SPAN.0 + 0x10;

const READER: u64 = 0x8000_0000_0024_590C;

fn memory() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem
}

/// A rule that keeps every stored word.
fn keep_value(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    words[index] = RegisterWord::known(store.value);
    true
}

#[test]
fn a_register_word_at_a_sourced_word_is_refused_and_declares_nothing() {
    let mut mem = memory();
    mem.declare_sourced_word(WORD, 1, READER).unwrap();
    let words = [
        (WORD - 4, RegisterWord::UNKNOWN),
        (WORD, RegisterWord::UNKNOWN),
    ];
    let error = mem
        .declare_registers(&words, keep_value)
        .expect_err("the memory accepts a register word at a sourced word");
    assert_eq!(
        (error, error.to_string()),
        (
            MemError::RegisterWordSourced { real: WORD },
            "the word at real 0x0000024003810010 for a register word is a sourced word already"
                .to_string(),
        )
    );
    assert_eq!(mem.register_word(WORD - 4), None);
    assert_eq!(mem.register_word(WORD), None);
    assert_eq!(
        mem.sourced_word(WORD, READER),
        Some((WORD, &1u32.to_be_bytes()[..]))
    );
}

#[test]
fn a_sourced_word_at_a_register_word_is_refused_and_records_nothing() {
    let mut mem = memory();
    mem.declare_registers(&[(WORD, RegisterWord::known(7))], keep_value)
        .unwrap();
    let error = mem
        .declare_sourced_word(WORD, 1, READER)
        .expect_err("the memory accepts a sourced word at a register word");
    assert_eq!(
        (error, error.to_string()),
        (
            MemError::SourcedWordRegistered {
                real: WORD,
                reader: READER,
            },
            "the word at real 0x0000024003810010 for a source for the reader \
             0x800000000024590c is a register word already"
                .to_string(),
        )
    );
    assert_eq!(mem.sourced_word(WORD, READER), None);
    assert_eq!(mem.register_word(WORD), Some(RegisterWord::known(7)));
}

#[test]
fn a_register_word_at_a_sourced_word_is_refused_as_sourced_before_its_value() {
    let mut mem = memory();
    mem.declare_sourced_word(WORD, 1, READER).unwrap();
    let loose = RegisterWord { value: 2, known: 1 };
    assert_eq!(
        mem.declare_registers(&[(WORD, loose)], keep_value),
        Err(MemError::RegisterWordSourced { real: WORD })
    );
}

#[test]
fn a_register_word_at_the_higher_of_two_sourced_words_is_refused_as_sourced() {
    let mut mem = memory();
    mem.declare_sourced_word(WORD, 1, READER).unwrap();
    mem.declare_sourced_word(WORD + 8, 2, READER).unwrap();
    assert_eq!(
        mem.declare_registers(&[(WORD + 8, RegisterWord::UNKNOWN)], keep_value),
        Err(MemError::RegisterWordSourced { real: WORD + 8 })
    );
    assert_eq!(mem.register_word(WORD + 8), None);
}

#[test]
fn sourced_words_and_register_words_at_adjacent_words_are_accepted() {
    let mut mem = memory();
    mem.declare_sourced_word(WORD, 1, READER).unwrap();
    let words = [
        (WORD - 4, RegisterWord::UNKNOWN),
        (WORD + 4, RegisterWord::UNKNOWN),
    ];
    assert_eq!(mem.declare_registers(&words, keep_value), Ok(()));
    assert_eq!(mem.declare_sourced_word(WORD - 8, 2, READER), Ok(()));
    assert_eq!(mem.declare_sourced_word(WORD + 8, 3, READER), Ok(()));
    assert_eq!(mem.register_word(WORD - 4), Some(RegisterWord::UNKNOWN));
    assert_eq!(mem.register_word(WORD + 4), Some(RegisterWord::UNKNOWN));
    assert_eq!(
        mem.sourced_word(WORD + 8, READER),
        Some((WORD + 8, &3u32.to_be_bytes()[..]))
    );
}
