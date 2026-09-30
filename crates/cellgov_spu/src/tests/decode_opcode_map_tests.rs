//! The decoder agrees with the ISA opcode map row by row, and classifies
//! the words it refuses by that map.

use super::*;
use crate::instruction::SpuInstructionKind;
use cellgov_ps3_abi::hw::spu_isa::SPU_OPCODE_MAP;

#[test]
fn every_row_decodes_as_its_own_mnemonic_or_is_refused_by_it() {
    for row in SPU_OPCODE_MAP {
        let word = row.canonical_word();
        match decode(word) {
            Ok(insn) => {
                assert!(
                    row.on_cbe,
                    "{} decodes though the CBE lacks it",
                    row.mnemonic
                );
                let kind: &'static str = SpuInstructionKind::from(insn).into();
                assert_eq!(kind.to_ascii_lowercase(), row.mnemonic, "0x{word:08x}");
            }
            Err(SpuDecodeError::Unimplemented { mnemonic, raw }) => {
                assert!(row.on_cbe, "{}", row.mnemonic);
                assert_eq!((mnemonic, raw), (row.mnemonic, word));
            }
            Err(SpuDecodeError::Unassigned(raw)) => {
                assert!(!row.on_cbe, "{} refused as unassigned", row.mnemonic);
                assert_eq!(raw, word);
            }
        }
    }
}

// [SPU-ISA p:259 s:A] no instruction has an RRR opcode of 1001 or 1010.
#[test]
fn a_word_no_row_owns_is_unassigned() {
    for word in [0x9000_0000, 0xA123_4567] {
        assert_eq!(decode(word), Err(SpuDecodeError::Unassigned(word)));
    }
}

#[test]
fn a_refused_word_keeps_its_operand_bits() {
    let (row, word) = SPU_OPCODE_MAP
        .iter()
        .map(|row| (row, row.canonical_word() | (u32::MAX >> row.width)))
        .find(|(row, word)| row.on_cbe && decode(*word).is_err())
        .expect("a CBE instruction without a decode arm");
    assert_eq!(
        decode(word),
        Err(SpuDecodeError::Unimplemented {
            raw: word,
            mnemonic: row.mnemonic
        })
    );
}
