use std::collections::BTreeSet;

use cellgov_ppu::instruction::fuzz::PpuFuzzKind;

use super::*;

/// `addi r3, r4, 5`: the D form with rt = 3 (bits 21 and 22), ra = 4 (bit
/// 18) and si = 5 (bits 0 and 2). Each rt and si bit clears to the same kind.
// [PPC-Book1 p:8 s:1.7.4] A D-form word is the six-bit primary opcode first,
// then two five-bit register fields (RT or RS, then RA), then a sixteen-bit
// immediate (SI, UI or D) in the last sixteen bits of the word.
// [PPC-Book1 p:203 s:Appendix J] Primary opcode 14 is addi and 24 is ori;
// both are D-form.
// [PPC-Book1 p:51 s:3.3.8] addi with RA = 0 loads the sign-extended
// immediate on its own, which the interpreter files under another kind, so
// the ra bit is not a same-kind candidate.
const ADDI_R3_R4_FIVE: u32 = (14 << 26) | (3 << 21) | (4 << 16) | 5;
const ORI_R5_R5_ONE: u32 = (24 << 26) | (5 << 21) | (5 << 16) | 1;

fn kind(raw: u32) -> PpuFuzzKind {
    cellgov_ppu::decode::decode(raw)
        .expect("word decodes")
        .fuzz_descriptor(raw)
        .kind
}

#[test]
fn instruction_shrinking_clears_one_operand_bit_of_the_first_word_only() {
    assert!(shrink_instruction_words(&[]).is_empty());

    let candidates = shrink_instruction_words(&[ADDI_R3_R4_FIVE, ORI_R5_R5_ONE]);

    let mut cleared = BTreeSet::new();
    for candidate in &candidates {
        let ReductionTransform::ClearOperandBit { index, bit } = candidate.transform else {
            panic!("instruction shrinking never drops a word: {candidate:?}");
        };
        assert_eq!(index, 0);
        assert_eq!(
            candidate.words,
            vec![ADDI_R3_R4_FIVE & !(1 << bit), ORI_R5_R5_ONE]
        );
        assert_eq!(kind(candidate.words[0]), kind(ADDI_R3_R4_FIVE));
        cleared.insert(bit);
    }
    for bit in [0, 2, 21, 22] {
        assert!(cleared.contains(&bit), "bit {bit} missing from {cleared:?}");
    }
}

#[test]
fn sequence_shrinking_drops_each_word_then_clears_operand_bits() {
    let words = [ADDI_R3_R4_FIVE, ORI_R5_R5_ONE];

    let candidates = shrink_sequence_words(&words);

    assert_eq!(
        candidates[..2],
        [
            ReductionCandidate {
                transform: ReductionTransform::DropWord { index: 0 },
                words: vec![ORI_R5_R5_ONE],
            },
            ReductionCandidate {
                transform: ReductionTransform::DropWord { index: 1 },
                words: vec![ADDI_R3_R4_FIVE],
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
    assert!(cleared.contains(&(0, 21)), "{cleared:?}");
}

#[test]
fn a_single_word_sequence_has_no_drop_candidate() {
    let candidates = shrink_sequence_words(&[ADDI_R3_R4_FIVE]);

    assert!(!candidates.is_empty());
    assert!(candidates.iter().all(|candidate| matches!(
        candidate.transform,
        ReductionTransform::ClearOperandBit { index: 0, .. }
    )));
}
