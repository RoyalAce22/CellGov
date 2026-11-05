//! A store that is not atomic still changes the word a later store in
//! its batch replaces.

use cellgov_exec::CodeEvent;

use super::tests::{fetch, run_one, step, store, NOP, X};

const LI_R3_1: u32 = 0x3860_0001;

/// Whether the one record is patch class, after a batch runs these steps:
///
/// 1. It runs `first`.
/// 2. It runs the full sequence on the line of `X`.
/// 3. It stores `nop` at `X`, then fetches from `X`.
fn patch_class_after(first: CodeEvent) -> bool {
    let (records, _) = run_one(&[step(&[
        first,
        CodeEvent::Dcbst { addr: X },
        CodeEvent::Sync,
        CodeEvent::Icbi { addr: X },
        CodeEvent::Isync,
        store(X, NOP),
        fetch(X),
    ])]);
    let [record] = records[..] else {
        panic!("one record, got {records:?}");
    };
    record.patch_class
}

#[test]
fn a_word_store_after_a_store_multiple_word_replaces_that_word() {
    let stmw_word = CodeEvent::Store {
        addr: X,
        len: 4,
        value: u128::from(LI_R3_1),
        atomic: false,
    };
    assert!(!patch_class_after(stmw_word));
}

#[test]
fn a_word_store_after_a_byte_store_replaces_the_merged_word() {
    // `b +8` with its first byte replaced by 0x38 is no patch-class word.
    let stb = CodeEvent::Store {
        addr: X,
        len: 1,
        value: 0x38,
        atomic: false,
    };
    assert!(!patch_class_after(stb));
}

#[test]
fn a_word_store_after_an_unaligned_store_that_straddles_two_words_replaces_the_merged_word() {
    // The bytes 00 48 38 00 land at `X - 2`, so `X` becomes 38 00 00 08,
    // which is no patch-class word.
    let stw_unaligned = CodeEvent::Store {
        addr: X - 2,
        len: 4,
        value: 0x0048_3800,
        atomic: false,
    };
    assert!(!patch_class_after(stw_unaligned));
}

#[test]
fn a_word_store_after_an_atomic_patch_store_stays_patch_class() {
    assert!(patch_class_after(store(X, NOP)));
}
