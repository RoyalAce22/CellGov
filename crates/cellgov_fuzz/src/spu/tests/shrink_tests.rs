use std::collections::BTreeSet;

use cellgov_spu::instruction::SpuInstructionKind;

use super::*;

/// `il r3, 5`: the RI16 form with rt = 3 (bits 0 and 1) and i16 = 5 (bits 7
/// and 9). The word has four set operand bits, and each one clears to the
/// same kind.
// [SPU-ISA p:52 s:Immediate Load Word] il is RI16: the nine-bit opcode
// 0x081 first, then the sixteen I16 bits, then RT in the low seven bits.
const IL_R3_FIVE: u32 = (0x081u32 << 23) | (5 << 7) | 3;
const NOP: u32 = 0x4020_007f;

fn kind(raw: u32) -> SpuInstructionKind {
    cellgov_spu::decode::decode(raw)
        .expect("word decodes")
        .fuzz_descriptor()
        .kind
}

#[test]
fn instruction_shrinking_clears_one_operand_bit_of_the_first_word_only() {
    assert!(shrink_instruction_words(&[]).is_empty());

    let candidates = shrink_instruction_words(&[IL_R3_FIVE, NOP]);

    let mut cleared = BTreeSet::new();
    for candidate in &candidates {
        let ReductionTransform::ClearOperandBit { index, bit } = candidate.transform else {
            panic!("instruction shrinking never drops a word: {candidate:?}");
        };
        assert_eq!(index, 0);
        assert_eq!(candidate.words, vec![IL_R3_FIVE & !(1 << bit), NOP]);
        assert_eq!(kind(candidate.words[0]), kind(IL_R3_FIVE));
        cleared.insert(bit);
    }
    assert_eq!(cleared, BTreeSet::from([0, 1, 7, 9]));
}

#[test]
fn sequence_shrinking_drops_each_word_then_clears_operand_bits() {
    let words = [IL_R3_FIVE, NOP];

    let candidates = shrink_sequence_words(&words);

    assert_eq!(
        candidates[..2],
        [
            ReductionCandidate {
                transform: ReductionTransform::DropWord { index: 0 },
                words: vec![NOP],
            },
            ReductionCandidate {
                transform: ReductionTransform::DropWord { index: 1 },
                words: vec![IL_R3_FIVE],
            },
        ]
    );
    let mut cleared = BTreeSet::new();
    for candidate in &candidates[2..] {
        let ReductionTransform::ClearOperandBit { index, bit } = candidate.transform else {
            panic!("drop candidates come before bit candidates: {candidate:?}");
        };
        let mut expected = words.to_vec();
        expected[index] &= !(1 << bit);
        assert_eq!(candidate.words, expected);
        assert_eq!(kind(candidate.words[index]), kind(words[index]));
        cleared.insert((index, bit));
    }
    assert!(cleared.contains(&(0, 9)), "{cleared:?}");
}

#[test]
fn a_single_word_sequence_has_no_drop_candidate() {
    let candidates = shrink_sequence_words(&[IL_R3_FIVE]);

    assert!(!candidates.is_empty());
    assert!(candidates.iter().all(|candidate| matches!(
        candidate.transform,
        ReductionTransform::ClearOperandBit { index: 0, .. }
    )));
}
