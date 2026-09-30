//! The catalog: every row, in identity order.

use super::rows_branch::BRANCH_ROWS;
use super::rows_compare::{
    CEQH_NOT_EQUAL_FUSED, CEQ_NOT_EQUAL_FUSED, CEQ_NOT_EQUAL_NOR, CEQ_NOT_EQUAL_RESULT_ONLY,
    SELECT_ROWS, SPLAT_ROWS,
};
use super::rows_float::{
    DIVISION_ROW, ESTIMATE_RECIPROCAL_ROW, ESTIMATE_RSQRT_ROW, NEWTON_RECIPROCAL_ONE_PLUS_ROW,
    NEWTON_RECIPROCAL_ROW, PICK_ROWS, RSQRT_NEWTON_ROW, SQUARE_ROOT_ROW,
};
use super::rows_integer::{FUNNEL_ROW, MPY32_ROW, MPY32_SWAPPED_ROW, NEGATED_ROWS, POPCOUNT_ROW};
use super::rows_memory::{
    MOVE_ORI_AI_ROW, MOVE_ORI_ANDI_ROW, MOVE_ORI_SHLQBYI_ROW, SPLIT_LOAD_ROW, SPLIT_STORE_ROW,
};
use super::rows_shuffle::INSERT_ROWS;
use super::types::SpuSequenceRelation;

/// The row groups, in identity order.
const GROUPS: [&[SpuSequenceRelation]; 13] = [
    &[
        CEQ_NOT_EQUAL_FUSED,
        CEQ_NOT_EQUAL_NOR,
        CEQ_NOT_EQUAL_RESULT_ONLY,
        CEQH_NOT_EQUAL_FUSED,
    ],
    &[MPY32_ROW, MPY32_SWAPPED_ROW],
    &SELECT_ROWS,
    &SPLAT_ROWS,
    &INSERT_ROWS,
    &NEGATED_ROWS,
    &[FUNNEL_ROW],
    &BRANCH_ROWS,
    &[POPCOUNT_ROW],
    &[SPLIT_LOAD_ROW, SPLIT_STORE_ROW],
    &[MOVE_ORI_AI_ROW, MOVE_ORI_ANDI_ROW, MOVE_ORI_SHLQBYI_ROW],
    &[
        ESTIMATE_RECIPROCAL_ROW,
        ESTIMATE_RSQRT_ROW,
        NEWTON_RECIPROCAL_ROW,
        NEWTON_RECIPROCAL_ONE_PLUS_ROW,
        RSQRT_NEWTON_ROW,
        SQUARE_ROOT_ROW,
        DIVISION_ROW,
    ],
    &PICK_ROWS,
];

const fn row_count() -> usize {
    let mut count = 0;
    let mut group = 0;
    while group < GROUPS.len() {
        count += GROUPS[group].len();
        group += 1;
    }
    count
}

const COUNT: usize = row_count();

const RELATIONS: [SpuSequenceRelation; COUNT] = {
    let mut out = [CEQ_NOT_EQUAL_FUSED; COUNT];
    let (mut at, mut group) = (0, 0);
    while group < GROUPS.len() {
        let mut row = 0;
        while row < GROUPS[group].len() {
            out[at] = GROUPS[group][row];
            at += 1;
            row += 1;
        }
        group += 1;
    }
    out
};

/// Every sequence relation, in identity order.
pub fn sequence_relations() -> &'static [SpuSequenceRelation] {
    &RELATIONS
}
