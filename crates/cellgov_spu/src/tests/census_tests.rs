//! Census counts by row and class.

use super::*;
use cellgov_ps3_abi::hw::spu_isa::{row_for, row_named};
use std::collections::BTreeSet;

fn canonical(mnemonic: &str) -> u32 {
    SPU_OPCODE_MAP[row_named(mnemonic).expect("the row exists")].canonical_word()
}

fn unassigned_word() -> u32 {
    (0..0x800u32)
        .map(|op| op << 21)
        .find(|&w| row_for(w).is_none())
        .expect("the opcode space has an unassigned word")
}

#[test]
fn each_word_counts_under_its_row_and_class() {
    let mut census = SpuCensus::new();
    for _ in 0..3 {
        census.add(canonical("il"));
    }
    census.add(canonical("dfceq"));
    census.add(unassigned_word());
    let rows: Vec<(&str, SpuWordClass, u64)> = census
        .rows()
        .map(|r| (r.row.mnemonic, r.class, r.words))
        .collect();
    assert_eq!(
        rows,
        [
            ("il", SpuWordClass::Implemented, 3),
            ("dfceq", SpuWordClass::AbsentOnCbe, 1),
        ]
    );
    assert_eq!(census.unassigned(), 1);
    assert_eq!(census.words(), 5);
    assert_eq!(census.words_in(SpuWordClass::Implemented), 3);
    assert_eq!(census.words_in(SpuWordClass::AbsentOnCbe), 1);
    assert_eq!(census.words_in(SpuWordClass::Unassigned), 1);
    assert_eq!(census.words_in(SpuWordClass::NotImplemented), 0);
}

#[test]
fn bytes_count_as_big_endian_words_and_a_partial_word_is_left_out() {
    let mut census = SpuCensus::new();
    let mut bytes = canonical("il").to_be_bytes().to_vec();
    bytes.extend_from_slice(&[0x40, 0x80]);
    census.add_bytes(&bytes);
    assert_eq!(census.words(), 1);
    assert_eq!(census.words_in(SpuWordClass::Implemented), 1);
}

#[test]
fn a_merge_adds_every_count() {
    let mut a = SpuCensus::new();
    a.add(canonical("il"));
    let mut b = SpuCensus::new();
    b.add(canonical("il"));
    b.add(unassigned_word());
    a.merge(&b);
    assert_eq!(a.words_in(SpuWordClass::Implemented), 2);
    assert_eq!(a.unassigned(), 1);
}

// -- images in a file: each counted once, executable segments only --

/// A one-segment SPU ELF whose PT_LOAD, with `flags`, holds `words`.
fn spu_elf(words: &[u32], flags: u32) -> Vec<u8> {
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let mut out = vec![0u8; 52 + 32];
    out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    out[4] = 1;
    out[5] = 2;
    out[18..20].copy_from_slice(&23u16.to_be_bytes());
    out[24..28].copy_from_slice(&0x100u32.to_be_bytes());
    out[28..32].copy_from_slice(&52u32.to_be_bytes());
    out[42..44].copy_from_slice(&32u16.to_be_bytes());
    out[44..46].copy_from_slice(&1u16.to_be_bytes());
    out[52..56].copy_from_slice(&1u32.to_be_bytes());
    out[56..60].copy_from_slice(&84u32.to_be_bytes());
    out[60..64].copy_from_slice(&0x100u32.to_be_bytes());
    let len = code.len() as u32;
    out[68..72].copy_from_slice(&len.to_be_bytes());
    out[72..76].copy_from_slice(&len.to_be_bytes());
    out[76..80].copy_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&code);
    out
}

/// `il rt, imm`.
const fn il(rt: u32, imm: u32) -> u32 {
    (0x081 << 23) | (imm << 7) | rt
}

#[test]
fn an_image_held_twice_counts_once() {
    let image = spu_elf(&[il(3, 1), il(4, 2)], PF_X);
    let mut host = image.clone();
    host.extend_from_slice(&image);
    let mut seen = BTreeSet::new();
    let mut total = SpuCensus::new();
    assert_eq!(total.add_new_images(&host, &mut seen), (1, 2));
    // A second file holding the same image adds nothing.
    assert_eq!(total.add_new_images(&image, &mut seen), (0, 0));
    assert_eq!(total.words(), 2);
}

#[test]
fn only_executable_segments_are_counted() {
    let data = spu_elf(&[il(3, 1), il(4, 2), il(5, 3)], 6);
    let mut seen = BTreeSet::new();
    let mut total = SpuCensus::new();
    assert_eq!(total.add_new_images(&data, &mut seen), (1, 0));
    assert_eq!(total.words(), 0);
}
