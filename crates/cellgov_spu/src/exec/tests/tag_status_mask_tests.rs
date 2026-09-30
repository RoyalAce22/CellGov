//! MFC_RdTagStat returns only the tag groups the query mask enables.

// [CBEA p:128 s:9.3.6] only the enabled tag groups' status is valid, and the bits of disabled groups are 0.

use super::*;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu;

fn read_tag_status(mask: u32, status: u32) -> (SpuStepOutcome, u32) {
    let mut s = SpuState::new();
    s.channels.tag_mask = mask;
    s.channels.tag_status = status;
    s.channels.request_tag_update(spu::MFC_TAG_UPDATE_IMMEDIATE);
    let out = execute(
        &SpuInstruction::Rdch {
            rt: 4,
            channel: spu::MFC_RD_TAG_STAT,
        },
        &mut s,
        UnitId::new(0),
    );
    (out, s.reg_word(4))
}

#[test]
fn a_completed_group_outside_the_mask_reads_zero() {
    // Groups 1 and 2 have completed; the query enables group 1 only.
    let (out, value) = read_tag_status(1 << 1, (1 << 1) | (1 << 2));
    assert_eq!(out, SpuStepOutcome::Continue);
    assert_eq!(value, 1 << 1);
}

#[test]
fn an_empty_query_mask_reads_zero_whatever_has_completed() {
    let (out, value) = read_tag_status(0, 0xFFFF_FFFF);
    assert_eq!(out, SpuStepOutcome::Continue);
    assert_eq!(value, 0);
}

#[test]
fn every_enabled_group_that_completed_reads_set() {
    let (out, value) = read_tag_status(0x8000_0003, 0xFFFF_FFFF);
    assert_eq!(out, SpuStepOutcome::Continue);
    assert_eq!(value, 0x8000_0003);
}
