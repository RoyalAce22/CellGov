//! Declaration refusals: the error and its message for each case in which
//! the memory refuses a sourced word, a register word, a reader or a
//! writer.

use super::*;
use crate::{PageSize, Region};

const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);

const WORD: u64 = SPAN.0 + 0x10;

const READER: u64 = 0x8000_0000_0024_590C;

const STORE: u64 = 0x8000_0000_0023_F964;

/// A span whose length is not a multiple of 4: the aligned word at
/// `SHORT.0 + 0x10` has only 3 of its bytes in it.
const SHORT: (u64, u64) = (0x240_0390_0000, 0x13);

/// The first of two adjacent spans. They meet inside the aligned word at
/// `SPLIT.0 + 0x10`.
const SPLIT: (u64, u64) = (0x240_03A0_0000, 0x12);

/// The base of a span that ends at `u64::MAX`, which it does not hold.
const TOP: u64 = u64::MAX - 0xF;

fn memory() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem
}

/// The memory with the spans of [`SHORT`], [`SPLIT`] and [`TOP`] as well.
fn with_boundary_spans() -> GuestMemory {
    let mut mem = memory();
    mem.declare_unmodelled(SHORT.0, SHORT.1).unwrap();
    mem.declare_unmodelled(SPLIT.0, SPLIT.1).unwrap();
    mem.declare_unmodelled(SPLIT.0 + SPLIT.1, 0xE).unwrap();
    mem.declare_unmodelled(TOP, u64::MAX - TOP).unwrap();
    mem
}

/// Aligned words that are not whole in one span: 3 bytes in a span, 2
/// bytes in each of two adjacent spans, and the word that holds the byte
/// at `u64::MAX`.
const SPLIT_WORDS: [u64; 3] = [SHORT.0 + 0x10, SPLIT.0 + 0x10, u64::MAX - 3];

/// Aligned words beside those of [`SPLIT_WORDS`] that one span holds whole.
const WHOLE_WORDS: [u64; 4] = [SHORT.0 + 0xC, SPLIT.0 + 0xC, SPLIT.0 + 0x14, u64::MAX - 7];

/// A rule that keeps every stored word.
fn keep_value(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    words[index] = RegisterWord::known(store.value);
    true
}

/// The memory with one register word at [`WORD`].
fn with_register() -> GuestMemory {
    let mut mem = memory();
    mem.declare_registers(&[(WORD, RegisterWord::UNKNOWN)], keep_value)
        .unwrap();
    mem
}

/// The error of a refused declaration, with its message.
fn refusal(declared: Result<(), MemError>) -> (MemError, String) {
    let error = declared.expect_err("the memory accepts the declaration");
    (error, error.to_string())
}

#[test]
fn a_misaligned_sourced_word_is_refused_as_unheld() {
    let mut mem = memory();
    assert_eq!(
        refusal(mem.declare_sourced_word(WORD + 2, 1, READER)),
        (
            MemError::SourcedWordUnheld {
                real: WORD + 2,
                reader: READER,
            },
            "no aligned word of one unmodelled span at real 0x0000024003810012 \
             for a source for the reader 0x800000000024590c"
                .to_string(),
        )
    );
}

#[test]
fn an_aligned_sourced_word_past_the_end_of_its_span_is_refused_as_unheld() {
    let mut mem = with_boundary_spans();
    let real = SHORT.0 + 0x10;
    assert_eq!(
        refusal(mem.declare_sourced_word(real, 1, READER)),
        (
            MemError::SourcedWordUnheld {
                real,
                reader: READER,
            },
            "no aligned word of one unmodelled span at real 0x0000024003900010 \
             for a source for the reader 0x800000000024590c"
                .to_string(),
        )
    );
}

#[test]
fn a_sourced_word_needs_all_four_bytes_in_one_span() {
    let mut mem = with_boundary_spans();
    for real in SPLIT_WORDS {
        assert_eq!(
            mem.declare_sourced_word(real, 1, READER),
            Err(MemError::SourcedWordUnheld {
                real,
                reader: READER,
            }),
            "{real:x}"
        );
    }
    for real in WHOLE_WORDS {
        assert_eq!(
            mem.declare_sourced_word(real, 1, READER),
            Ok(()),
            "{real:x}"
        );
    }
}

#[test]
fn a_register_word_needs_all_four_bytes_in_one_span() {
    let mut mem = with_boundary_spans();
    for real in SPLIT_WORDS {
        assert_eq!(
            mem.declare_registers(&[(real, RegisterWord::UNKNOWN)], keep_value),
            Err(MemError::RegisterWordUnheld { real }),
            "{real:x}"
        );
    }
    for real in WHOLE_WORDS {
        assert_eq!(
            mem.declare_registers(&[(real, RegisterWord::UNKNOWN)], keep_value),
            Ok(()),
            "{real:x}"
        );
    }
}

#[test]
fn the_first_refused_word_of_a_declaration_names_the_refusal() {
    let mut mem = memory();
    let loose = RegisterWord { value: 2, known: 1 };
    let words = [(WORD, loose), (WORD + 2, RegisterWord::UNKNOWN)];
    assert_eq!(
        mem.declare_registers(&words, keep_value),
        Err(MemError::RegisterValueOutsideKnown { real: WORD })
    );
    assert_eq!(mem.register_word(WORD), None);
}

#[test]
fn an_unheld_or_repeated_register_word_is_refused_as_such_before_its_value() {
    let mut mem = with_register();
    let loose = RegisterWord { value: 2, known: 1 };
    assert_eq!(
        mem.declare_registers(&[(WORD + 2, loose)], keep_value),
        Err(MemError::RegisterWordUnheld { real: WORD + 2 })
    );
    assert_eq!(
        mem.declare_registers(&[(WORD, loose)], keep_value),
        Err(MemError::RegisterWordRepeated { real: WORD })
    );
}

#[test]
fn a_repeated_sourced_word_and_reader_is_refused_as_repeated() {
    let mut mem = memory();
    mem.declare_sourced_word(WORD, 1, READER).unwrap();
    assert_eq!(
        refusal(mem.declare_sourced_word(WORD, 2, READER)),
        (
            MemError::SourcedWordRepeated {
                real: WORD,
                reader: READER,
            },
            "the sourced word at real 0x0000024003810010 for the reader \
             0x800000000024590c is declared twice"
                .to_string(),
        )
    );
}

#[test]
fn a_misaligned_register_word_is_refused_as_unheld() {
    let mut mem = memory();
    let words = [(WORD + 2, RegisterWord::UNKNOWN)];
    assert_eq!(
        refusal(mem.declare_registers(&words, keep_value)),
        (
            MemError::RegisterWordUnheld { real: WORD + 2 },
            "no aligned word of one unmodelled span at real 0x0000024003810012 \
             for a register word"
                .to_string(),
        )
    );
}

#[test]
fn a_register_word_outside_every_span_is_refused_as_unheld() {
    let mut mem = memory();
    let real = SPAN.0 + SPAN.1;
    let words = [(real, RegisterWord::UNKNOWN)];
    assert_eq!(
        refusal(mem.declare_registers(&words, keep_value)),
        (
            MemError::RegisterWordUnheld { real },
            "no aligned word of one unmodelled span at real 0x0000024003820000 \
             for a register word"
                .to_string(),
        )
    );
}

#[test]
fn a_register_word_twice_in_one_declaration_is_refused_as_repeated() {
    let mut mem = memory();
    let words = [(WORD, RegisterWord::UNKNOWN), (WORD, RegisterWord::UNKNOWN)];
    assert_eq!(
        refusal(mem.declare_registers(&words, keep_value)),
        (
            MemError::RegisterWordRepeated { real: WORD },
            "the register word at real 0x0000024003810010 is declared twice".to_string(),
        )
    );
}

#[test]
fn a_register_word_that_the_memory_holds_is_refused_as_repeated() {
    let mut mem = with_register();
    let words = [(WORD, RegisterWord::UNKNOWN)];
    assert_eq!(
        refusal(mem.declare_registers(&words, keep_value)),
        (
            MemError::RegisterWordRepeated { real: WORD },
            "the register word at real 0x0000024003810010 is declared twice".to_string(),
        )
    );
}

#[test]
fn a_register_value_with_an_unknown_bit_set_is_refused_as_outside_known() {
    let mut mem = memory();
    let words = [(WORD, RegisterWord { value: 2, known: 1 })];
    assert_eq!(
        refusal(mem.declare_registers(&words, keep_value)),
        (
            MemError::RegisterValueOutsideKnown { real: WORD },
            "the register word at real 0x0000024003810010 has a value bit \
             outside its known bits"
                .to_string(),
        )
    );
}

#[test]
fn a_reader_of_no_register_word_is_refused_as_no_register_word() {
    let mut mem = memory();
    assert_eq!(
        refusal(mem.declare_register_reader(WORD, READER, 0, None)),
        (
            MemError::NoRegisterWord { real: WORD },
            "no register word at real 0x0000024003810010 for a reader or a writer".to_string(),
        )
    );
}

#[test]
fn a_repeated_reader_of_a_register_word_is_refused_as_repeated() {
    let mut mem = with_register();
    mem.declare_register_reader(WORD, READER, 0, None).unwrap();
    assert_eq!(
        refusal(mem.declare_register_reader(WORD, READER, 0, None)),
        (
            MemError::RegisterReaderRepeated {
                real: WORD,
                reader: READER,
            },
            "the instruction at 0x800000000024590c is declared twice as a reader \
             of the register word at real 0x0000024003810010"
                .to_string(),
        )
    );
}

#[test]
fn a_writer_of_no_register_word_is_refused_as_no_register_word() {
    let mut mem = memory();
    assert_eq!(
        refusal(mem.declare_register_writer(WORD, STORE)),
        (
            MemError::NoRegisterWord { real: WORD },
            "no register word at real 0x0000024003810010 for a reader or a writer".to_string(),
        )
    );
}

#[test]
fn a_repeated_writer_of_a_register_word_is_refused_as_repeated() {
    let mut mem = with_register();
    mem.declare_register_writer(WORD, STORE).unwrap();
    assert_eq!(
        refusal(mem.declare_register_writer(WORD, STORE)),
        (
            MemError::RegisterWriterRepeated {
                real: WORD,
                store: STORE,
            },
            "the store instruction at 0x800000000023f964 is declared twice as a writer \
             of the register word at real 0x0000024003810010"
                .to_string(),
        )
    );
}
