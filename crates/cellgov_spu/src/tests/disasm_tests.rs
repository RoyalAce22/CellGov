//! Word text by form, and the class the decoder gives each word.

use super::*;
use cellgov_ps3_abi::hw::spu_isa::{row_named, SPU_OPCODE_MAP};

#[test]
fn a_word_names_its_mnemonic_and_its_forms_fields() {
    // il $3,0x5
    let il = SpuWord::of((0x081 << 23) | (5 << 7) | 3);
    assert_eq!(il.to_string(), "il       $3,0x5");
    assert_eq!(il.class, SpuWordClass::Implemented);
    // shufb $4,$1,$2,$3: RT in the high field, RC in the low one.
    let shufb = SpuWord::of(0xB000_0000 | (4 << 21) | (2 << 14) | (1 << 7) | 3);
    assert_eq!(shufb.to_string(), "shufb    $4,$1,$2,$3");
    // a $5,$6,$7
    let a = SpuWord::of((0b00011000000 << 21) | (7 << 14) | (6 << 7) | 5);
    assert_eq!(a.to_string(), "a        $5,$6,$7");
}

#[test]
fn an_immediate_prints_its_raw_field() {
    // ai $3,$4,-1: the ten-bit field is all ones.
    let ai = SpuWord::of((0b00011100 << 24) | (0x3ff << 14) | (4 << 7) | 3);
    assert_eq!(ai.to_string(), "ai       $3,$4,0x3ff");
}

#[test]
fn a_hint_prints_its_branch_offset_then_its_target() {
    let hbra = SPU_OPCODE_MAP[row_named("hbra").expect("hbra has a row")];
    let raw = hbra.canonical_word() | (0b10 << 23) | (0x1234 << 7) | 0x05;
    assert_eq!(SpuWord::of(raw).to_string(), "hbra     0x105,0x1234");
}

#[test]
fn a_word_no_row_selects_is_a_data_word() {
    let raw = (0..0x800u32)
        .map(|op| op << 21)
        .find(|&w| spu_isa::row_for(w).is_none())
        .expect("the opcode space has an unassigned word");
    let word = SpuWord::of(raw);
    assert_eq!(word.class, SpuWordClass::Unassigned);
    assert_eq!(word.to_string(), format!(".word 0x{raw:08x}"));
}

#[test]
fn every_row_has_the_class_the_decoder_gives_it() {
    for row in SPU_OPCODE_MAP {
        let word = SpuWord::of(row.canonical_word());
        let want = match crate::decode::decode(row.canonical_word()) {
            Ok(_) => SpuWordClass::Implemented,
            Err(_) if !row.on_cbe => SpuWordClass::AbsentOnCbe,
            Err(_) => SpuWordClass::NotImplemented,
        };
        assert_eq!(word.class, want, "{}", row.mnemonic);
        assert_eq!(word.row.map(|(_, r)| r.mnemonic), Some(row.mnemonic));
    }
    let dfceq = SPU_OPCODE_MAP[row_named("dfceq").expect("dfceq has a row")];
    assert_eq!(
        SpuWord::of(dfceq.canonical_word()).class,
        SpuWordClass::AbsentOnCbe
    );
}

#[test]
fn every_decoded_row_names_its_own_mnemonic() {
    for row in SPU_OPCODE_MAP {
        assert!(agrees_with_decode(row.canonical_word()), "{}", row.mnemonic);
    }
}

#[test]
fn every_field_the_decoder_reads_is_a_field_the_text_shows() {
    // The decoder reads each row's operands at fixed bit positions, so
    // the row's canonical word stands for every word of the row.
    let mut checked = 0;
    for row in SPU_OPCODE_MAP {
        let Some(descriptor) = crate::fuzz::generation_descriptor(row.canonical_word()) else {
            continue;
        };
        checked += 1;
        let read = descriptor
            .operands
            .iter()
            .fold(0u32, |mask, field| mask | field.mask);
        let shown = SpuWord::of(row.canonical_word()).rendered_field_mask();
        assert_eq!(
            read & !shown,
            0,
            "{}: the decoder reads bits {:#010x} the text does not show",
            row.mnemonic,
            read & !shown
        );
    }
    let implemented = SPU_OPCODE_MAP
        .iter()
        .filter(|row| crate::decode::decode(row.canonical_word()).is_ok())
        .count();
    assert_eq!(
        checked, implemented,
        "every implemented row has a descriptor"
    );
}
