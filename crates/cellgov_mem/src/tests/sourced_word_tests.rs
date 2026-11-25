//! Sourced words: which word a lookup gives each reader, and which
//! declarations the memory refuses.

use super::*;
use crate::{PageSize, Region};

/// A 16 KB region at 0 and a 64 KB span at 0x24003810000.
const SPAN: (u64, u64) = (0x240_0381_0000, 0x1_0000);

const READER: u64 = 0x8000_0000_0024_590C;

fn memory() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem
}

#[test]
fn a_sourced_word_answers_each_of_its_bytes_for_its_reader_only() {
    let mut mem = memory();
    let word = SPAN.0 + 4;
    mem.declare_sourced_word(word, 0x0102_0304, READER).unwrap();
    for byte in word..word + 4 {
        assert_eq!(
            mem.sourced_word(byte, READER),
            Some((word, &[1u8, 2, 3, 4][..]))
        );
    }
    assert_eq!(mem.sourced_word(word, READER + 4), None);
    assert_eq!(mem.sourced_word(word - 1, READER), None);
    assert_eq!(mem.sourced_word(word + 4, READER), None);
}

#[test]
fn the_first_and_the_last_word_of_a_span_are_held() {
    let mut mem = memory();
    let (base, len) = SPAN;
    let last = base + len - 4;
    mem.declare_sourced_word(base, 0x0A0B_0C0D, READER).unwrap();
    mem.declare_sourced_word(last, 0x1121_3141, READER).unwrap();
    assert_eq!(
        mem.sourced_word(base, READER),
        Some((base, &[0x0Au8, 0x0B, 0x0C, 0x0D][..]))
    );
    assert_eq!(
        mem.sourced_word(last + 3, READER),
        Some((last, &[0x11u8, 0x21, 0x31, 0x41][..]))
    );
}

#[test]
fn two_readers_of_one_word_get_their_own_values() {
    let mut mem = memory();
    mem.declare_sourced_word(SPAN.0, 0x10, READER + 4).unwrap();
    mem.declare_sourced_word(SPAN.0, 0x20, READER).unwrap();
    assert_eq!(
        mem.sourced_word(SPAN.0, READER),
        Some((SPAN.0, &[0u8, 0, 0, 0x20][..]))
    );
    assert_eq!(
        mem.sourced_word(SPAN.0, READER + 4),
        Some((SPAN.0, &[0u8, 0, 0, 0x10][..]))
    );
}

#[test]
fn a_misaligned_word_a_word_outside_one_span_and_a_repeat_are_refused() {
    let mut mem = memory();
    let (base, len) = SPAN;
    let refused = |real| {
        Err(MemError::SourcedWordRefused {
            real,
            reader: READER,
        })
    };
    assert_eq!(
        mem.declare_sourced_word(base + 2, 1, READER),
        refused(base + 2)
    );
    assert_eq!(
        mem.declare_sourced_word(base + len - 2, 1, READER),
        refused(base + len - 2)
    );
    assert_eq!(
        mem.declare_sourced_word(base + len, 1, READER),
        refused(base + len)
    );
    assert_eq!(mem.declare_sourced_word(0, 1, READER), refused(0));
    mem.declare_sourced_word(base, 1, READER).unwrap();
    assert_eq!(mem.declare_sourced_word(base, 2, READER), refused(base));
    assert_eq!(
        mem.sourced_word(base, READER),
        Some((base, &[0u8, 0, 0, 1][..]))
    );
    assert_eq!(mem.sourced_word(base + len - 4, READER), None);
}
