//! The word lines `dev spu-disasm` writes, their notes, and the words it
//! counts as no instruction the CBE runs.

use super::*;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SPU_OPCODE_MAP};

fn canonical(mnemonic: &str) -> u32 {
    SPU_OPCODE_MAP[row_named(mnemonic).expect("the row exists")].canonical_word()
}

fn bytes_of(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn render(bytes: &[u8], lsa: u32, start: u32, count: usize) -> (String, usize) {
    let mut out = Vec::new();
    let invalid = write_words(&mut out, &Span { lsa, bytes }, start, count).expect("writes");
    (String::from_utf8(out).expect("ascii"), invalid)
}

#[test]
fn each_line_is_the_address_the_word_and_its_text() {
    let il = (0x081 << 23) | (5 << 7) | 3;
    let (text, invalid) = render(&bytes_of(&[il, canonical("dfceq")]), 0x100, 0x100, 2);
    assert_eq!(
        text,
        "0x00100  40800283  il       $3,0x5\n\
         0x00104  78600000  dfceq    $0,$0,$0  ; absent on the CBE\n"
    );
    assert_eq!(
        invalid, 1,
        "a word absent on the CBE is no instruction it runs"
    );
}

#[test]
fn a_start_inside_the_span_skips_the_words_before_it() {
    let words = bytes_of(&[canonical("il"), canonical("ai"), canonical("a")]);
    let (text, _) = render(&words, 0x40, 0x48, 1);
    assert!(text.starts_with("0x00048  "), "{text}");
    assert!(text.contains("  a        $0,$0,$0"), "{text}");
}

#[test]
fn the_span_end_is_marked_and_a_data_word_counts_as_invalid() {
    let data = (0..0x800u32)
        .map(|op| op << 21)
        .find(|&w| cellgov_ps3_abi::hw::spu_isa::row_for(w).is_none())
        .expect("the opcode space has an unassigned word");
    let (text, invalid) = render(&bytes_of(&[data]), 0, 0, 3);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].ends_with(&format!(".word 0x{data:08x}")), "{text}");
    assert_eq!(lines[1], "0x00004  --------  <past segment end>");
    assert_eq!(invalid, 1);
}
