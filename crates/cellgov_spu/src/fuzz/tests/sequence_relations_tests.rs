use super::*;

use crate::state::SpuState;

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
fn every_row_is_in_identity_order_and_counts_its_symbolic_registers() {
    let ids: Vec<_> = sequence_relations().iter().map(|row| row.id).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(ids, sorted);
    for row in sequence_relations() {
        assert_eq!(row.register_count(), 4, "{:?}", row.id);
    }
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
    (fused.apply)(&mut state, &[9, 4, 5, 4]);
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
