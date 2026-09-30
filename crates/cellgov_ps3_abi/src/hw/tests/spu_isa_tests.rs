//! The opcode map is prefix-free, so a word matches at most one row.

use super::*;

#[test]
fn no_opcode_is_a_prefix_of_another() {
    for a in SPU_OPCODE_MAP {
        for b in SPU_OPCODE_MAP {
            if a.mnemonic != b.mnemonic && a.width <= b.width {
                let b_prefix = b.opcode >> (b.width - a.width);
                assert_ne!(
                    a.opcode, b_prefix,
                    "{} is a prefix of {}",
                    a.mnemonic, b.mnemonic
                );
            }
        }
    }
}

// [SPU-ISA p:259 s:A] Table A-1 lists 199 SPU instructions.
#[test]
fn every_row_finds_itself_by_its_canonical_word() {
    assert_eq!(SPU_OPCODE_MAP.len(), 199);
    for (index, row) in SPU_OPCODE_MAP.iter().enumerate() {
        assert_eq!(row_for(row.canonical_word()), Some((index, row)));
    }
}

// [SPU-ISA p:259 s:A] no instruction has an RRR opcode of 1001 or 1010.
#[test]
fn a_word_no_row_owns_finds_none() {
    assert_eq!(row_for(0x9000_0000), None);
    assert_eq!(row_for(0xA123_4567), None);
}
