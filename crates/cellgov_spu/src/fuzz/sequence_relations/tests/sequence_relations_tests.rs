use super::lanes::{compare_mask, float_key, Compare, Width};
use super::rows_compare::CEQ_NOT_EQUAL;
use super::*;

use crate::instruction::SpuInstructionKind;
use crate::state::SpuState;

/// Symbolic `c` and `rt` of the negated-compare rows.
const C: u8 = 0;
const RT: u8 = 3;

fn relation(id: SpuSequenceRelationId) -> SpuSequenceRelation {
    *sequence_relations()
        .iter()
        .find(|row| row.id == id)
        .expect("every id has a row")
}

#[test]
fn a_symbolic_word_encodes_to_the_decoded_instruction_under_its_assignment() {
    let assignment = [9, 4, 5, 70];
    let words: Vec<u32> = CEQ_NOT_EQUAL
        .iter()
        .map(|word| {
            word.encode(&assignment)
                .expect("the rows use encodable forms")
        })
        .collect();
    assert_eq!(
        crate::decode::decode(words[0]),
        Ok(crate::instruction::SpuInstruction::Ceq {
            rt: 9,
            ra: 4,
            rb: 5
        })
    );
    assert_eq!(
        crate::decode::decode(words[1]),
        Ok(crate::instruction::SpuInstruction::Ceqi {
            rt: 70,
            ra: 9,
            imm: 0
        })
    );
    assert_eq!(
        CEQ_NOT_EQUAL[0].encode(&[1, 2]),
        None,
        "too short an assignment"
    );
}

#[test]
fn every_row_is_in_identity_order_and_every_word_decodes_to_its_kind() {
    let ids: Vec<_> = sequence_relations().iter().map(|row| row.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids, sorted);
    assert_eq!(ids.len(), 56);
    for row in sequence_relations() {
        let assignment: Vec<u8> = (10..10 + row.register_count() as u8).collect();
        let partner: &[SpuSymbolicWord] = match row.partner {
            SpuSequencePartner::Guest(words) => words,
            SpuSequencePartner::Fused(_) => &[],
        };
        for word in row.sequence.iter().chain(partner) {
            let raw = word
                .encode(&assignment)
                .unwrap_or_else(|| panic!("{:?}: {:?} does not encode", row.id, word.kind));
            let decoded = crate::decode::decode(raw)
                .unwrap_or_else(|_| panic!("{:?}: {raw:#010x} does not decode", row.id));
            assert_eq!(
                SpuInstructionKind::from(decoded),
                word.kind,
                "{:?}: {raw:#010x}",
                row.id
            );
        }
    }
}

#[test]
fn a_relative_branch_to_the_landing_targets_it_from_its_own_slot() {
    let word = super::types::branch_to_landing(SpuInstructionKind::Brz, 0, 1);
    let raw = word.encode(&[7]).expect("brz encodes");
    let crate::instruction::SpuInstruction::Brz { rt, offset } =
        crate::decode::decode(raw).expect("brz decodes")
    else {
        panic!("the word is a brz");
    };
    assert_eq!(rt, 7);
    let from = SEQUENCE_PROGRAM_BASE + 4;
    assert_eq!(
        from.wrapping_add((offset * 4) as u32),
        SEQUENCE_TAKEN_LANDING
    );
}

#[test]
fn single_precision_order_treats_every_zero_exponent_as_zero() {
    // A denorm, positive zero and negative zero are all zero.
    assert_eq!(float_key(0x0000_0001), 0);
    assert_eq!(float_key(0x8000_0000), 0);
    assert_eq!(float_key(0x807F_FFFF), 0);
    // Exponent 255 is an ordinary number in the extended range.
    assert!(float_key(0x7F80_0000) > float_key(0x7F00_0000));
    assert!(float_key(0xBF80_0000) < 0);
    let words = |values: [u32; 4]| -> [u8; 16] {
        std::array::from_fn(|byte| values[byte / 4].to_be_bytes()[byte % 4])
    };
    let x = words([0x0000_0001, 0x3F80_0000, 0xBF80_0000, 0x4000_0000]);
    let y = words([0x8000_0000, 0x3F80_0000, 0x3F80_0000, 0x3F80_0000]);
    let equal = compare_mask(Compare::FloatEqual, Width::Word, x, y);
    assert_eq!(equal, words([u32::MAX, u32::MAX, 0, 0]));
    let magnitude = compare_mask(Compare::MagnitudeEqual, Width::Word, x, y);
    assert_eq!(magnitude, words([u32::MAX, u32::MAX, u32::MAX, 0]));
    let greater = compare_mask(Compare::FloatGreater, Width::Word, x, y);
    assert_eq!(greater, words([0, 0, 0, u32::MAX]));
}

#[test]
fn the_fused_compare_writes_the_mask_and_its_complement_from_the_start_values() {
    let SpuSequencePartner::Fused(fused) =
        relation(SpuSequenceRelationId::CeqNotEqualFused).partner
    else {
        panic!("the fused row has a fused partner");
    };
    let mut state = SpuState::new();
    state.set_reg(4, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    state.set_reg(5, [1, 2, 3, 4, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0]);
    // RT aliases A: the result still follows the start value of A.
    assert_eq!(
        (fused.apply)(&mut state, &[9, 4, 5, 4]),
        SpuFusedFlow::FallThrough
    );
    let mut mask = [0u8; 16];
    mask[0..4].fill(0xFF);
    mask[8..12].fill(0xFF);
    assert_eq!(state.regs[9], mask);
    assert_eq!(state.regs[4], mask.map(|byte| !byte));
    assert_eq!(fused.writes, [C, RT]);
}

#[test]
fn a_dead_register_stays_compared_when_it_shares_a_live_result_register() {
    let row = relation(SpuSequenceRelationId::CeqNotEqualResultOnly);
    assert_eq!(row.dead, [C]);
    // Distinct registers: c is left out.
    assert_eq!(row.excluded_registers(&[9, 4, 5, 70], row.dead), [9]);
    // c aliases the input A: after the sequence that register holds c.
    assert_eq!(row.excluded_registers(&[4, 4, 5, 70], row.dead), [4]);
    // c aliases RT, a live result: the register stays compared.
    assert_eq!(row.excluded_registers(&[70, 4, 5, 70], row.dead), [0u8; 0]);
    // An empty dead set leaves nothing out.
    assert_eq!(row.excluded_registers(&[9, 4, 5, 70], &[]), [0u8; 0]);
}

#[test]
fn the_result_only_partner_writes_rt_and_leaves_c_as_it_was() {
    let SpuSequencePartner::Fused(fused) =
        relation(SpuSequenceRelationId::CeqNotEqualResultOnly).partner
    else {
        panic!("the result-only row has a fused partner");
    };
    let mut state = SpuState::new();
    state.set_reg(9, [0x77; 16]);
    state.set_reg(4, [1; 16]);
    state.set_reg(5, [1; 16]);
    (fused.apply)(&mut state, &[9, 4, 5, 70]);
    assert_eq!(state.regs[9], [0x77; 16], "c kept");
    assert_eq!(state.regs[70], [0; 16], "equal words give zero");
    assert_eq!(fused.writes, [RT]);
}
