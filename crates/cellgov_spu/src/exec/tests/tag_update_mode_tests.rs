//! MFC_WrTagUpdate's three update conditions, MFC_RdTagMask, the empty
//! query mask, and a tag-status read with no update request.

// [CBE-Handbook p:459 s:17.10] TS 00 updates immediately, 01 when any enabled group completes, 10 when all do; 11 is reserved.
// [CBEA p:128 s:9.3.6] the status bits of groups the query mask leaves out read 0.

use super::*;
use crate::exec::SpuFault;
use crate::state::{SpuState, TagUpdateCondition};
use cellgov_ps3_abi::hw::spu;

fn request(s: &mut SpuState, ts: u32) {
    s.set_reg_word_splat(7, ts);
    let out = execute(
        &SpuInstruction::Wrch {
            channel: spu::MFC_WR_TAG_UPDATE,
            rt: 7,
        },
        s,
        UnitId::new(0),
    );
    assert_eq!(out, SpuStepOutcome::Continue);
}

fn read(s: &mut SpuState) -> SpuStepOutcome {
    execute(
        &SpuInstruction::Rdch {
            rt: 4,
            channel: spu::MFC_RD_TAG_STAT,
        },
        s,
        UnitId::new(0),
    )
}

fn state(mask: u32, status: u32) -> SpuState {
    let mut s = SpuState::new();
    s.channels.tag_mask = mask;
    s.channels.tag_status = status;
    s
}

#[test]
fn an_immediate_request_reads_the_masked_status_without_waiting() {
    // Groups 1 and 2 are enabled; only group 1 is complete.
    let mut s = state(0b110, 0b010);
    request(&mut s, spu::MFC_TAG_UPDATE_IMMEDIATE);
    assert_eq!(read(&mut s), SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(4), 0b010);
}

#[test]
fn an_any_request_waits_for_one_enabled_group_then_latches() {
    let mut s = state(0b110, 0);
    request(&mut s, spu::MFC_TAG_UPDATE_ANY);
    assert_eq!(s.channels.tag_update, Some(TagUpdateCondition::Any));
    assert!(matches!(read(&mut s), SpuStepOutcome::Yield { .. }));
    s.channels.tag_status = 0b100;
    s.channels.settle_tag_update();
    assert_eq!(read(&mut s), SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(4), 0b100);
}

#[test]
fn an_all_request_waits_for_every_enabled_group() {
    let mut s = state(0b110, 0b010);
    request(&mut s, spu::MFC_TAG_UPDATE_ALL);
    assert!(matches!(read(&mut s), SpuStepOutcome::Yield { .. }));
    s.channels.tag_status = 0b111;
    s.channels.settle_tag_update();
    assert_eq!(read(&mut s), SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(4), 0b110);
}

#[test]
fn a_request_met_when_written_latches_at_once() {
    let mut s = state(0b110, 0b110);
    request(&mut s, spu::MFC_TAG_UPDATE_ALL);
    assert_eq!(s.channels.tag_update, None);
    assert_eq!(s.channels.tag_status_read, Some(0b110));
}

#[test]
fn with_an_empty_mask_all_latches_zero_and_any_waits() {
    let mut s = state(0, u32::MAX);
    request(&mut s, spu::MFC_TAG_UPDATE_ALL);
    assert_eq!(read(&mut s), SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(4), 0);

    let mut s = state(0, u32::MAX);
    request(&mut s, spu::MFC_TAG_UPDATE_ANY);
    s.channels.settle_tag_update();
    assert!(matches!(read(&mut s), SpuStepOutcome::Yield { .. }));
}

#[test]
fn the_status_keeps_the_mask_of_its_update() {
    let mut s = state(0b010, 0b111);
    request(&mut s, spu::MFC_TAG_UPDATE_IMMEDIATE);
    s.channels.tag_mask = 0b111;
    assert_eq!(read(&mut s), SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(4), 0b010);
}

#[test]
fn a_new_request_replaces_an_unread_result() {
    let mut s = state(0b010, 0b010);
    request(&mut s, spu::MFC_TAG_UPDATE_IMMEDIATE);
    s.channels.tag_status = 0;
    request(&mut s, spu::MFC_TAG_UPDATE_ALL);
    assert_eq!(s.channels.tag_status_read, None);
    assert!(matches!(read(&mut s), SpuStepOutcome::Yield { .. }));
}

// [CBEA p:128 s:9.3.5] an immediate request cancels a waiting conditional request.
#[test]
fn an_immediate_request_cancels_a_waiting_conditional_one() {
    // Group 1 is enabled and still queued.
    let mut s = state(0b010, 0);
    request(&mut s, spu::MFC_TAG_UPDATE_ALL);
    assert_eq!(s.channels.tag_update, Some(TagUpdateCondition::All));
    request(&mut s, spu::MFC_TAG_UPDATE_IMMEDIATE);
    assert_eq!(s.channels.tag_update, None);
    assert_eq!(read(&mut s), SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(4), 0);
    // The group completes later; the cancelled request latches nothing.
    s.channels.tag_status = 0b010;
    s.channels.settle_tag_update();
    assert_eq!(s.channels.tag_status_read, None);
}

#[test]
fn a_read_with_no_request_is_refused_by_name() {
    let mut s = state(0b010, 0b010);
    assert_eq!(
        read(&mut s),
        SpuStepOutcome::Fault(SpuFault::ChannelStall(spu::MFC_RD_TAG_STAT))
    );
}

#[test]
fn a_reserved_update_request_is_refused_by_name() {
    for value in [3, 1 << 2, 0x8000_0001] {
        let mut s = state(0b010, 0b010);
        s.set_reg_word_splat(7, value);
        let out = execute(
            &SpuInstruction::Wrch {
                channel: spu::MFC_WR_TAG_UPDATE,
                rt: 7,
            },
            &mut s,
            UnitId::new(0),
        );
        assert_eq!(
            out,
            SpuStepOutcome::Fault(SpuFault::ReservedTagUpdate(value)),
            "value {value:#x}"
        );
    }
}

#[test]
fn a_request_that_never_reaches_the_channel_leaves_no_state() {
    let mut s = state(0b010, 0b010);
    s.channels.request_tag_update(3);
    assert_eq!(s.channels.tag_update, None);
    assert_eq!(s.channels.tag_status_read, None);
    assert_eq!(
        read(&mut s),
        SpuStepOutcome::Fault(SpuFault::ChannelStall(spu::MFC_RD_TAG_STAT))
    );
}

/// Runs one step of a unit whose first word is `stop`, with `outstanding`
/// tag groups still queued, and returns its latched status.
fn step_entry(condition: TagUpdateCondition, mask: u32, outstanding: u32) -> Option<u32> {
    use crate::SpuExecutionUnit;
    use cellgov_exec::{ExecutionContext, ExecutionUnit};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    let mut unit = SpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().channels.tag_mask = mask;
    unit.state_mut().channels.tag_update = Some(condition);
    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem).with_outstanding_dma_tags(outstanding);
    unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    unit.state().channels.tag_status_read
}

#[test]
fn a_waiting_request_settles_at_step_entry_from_the_outstanding_groups() {
    // Group 2 is still queued; group 1 landed.
    assert_eq!(
        step_entry(TagUpdateCondition::Any, 0b110, 0b100),
        Some(0b010)
    );
    assert_eq!(step_entry(TagUpdateCondition::All, 0b110, 0b100), None);
    assert_eq!(step_entry(TagUpdateCondition::All, 0b110, 0), Some(0b110));
}

// [CBEA p:126 s:9.3.4] MFC_RdTagMask returns the current query mask, and its count is always 1.
#[test]
fn the_query_mask_reads_back_and_counts_one() {
    let mut s = state(0x8000_0011, 0);
    let out = execute(
        &SpuInstruction::Rdch {
            rt: 4,
            channel: spu::MFC_RD_TAG_MASK,
        },
        &mut s,
        UnitId::new(0),
    );
    assert_eq!(out, SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(4), 0x8000_0011);
    execute(
        &SpuInstruction::Rchcnt {
            rt: 5,
            channel: spu::MFC_RD_TAG_MASK,
        },
        &mut s,
        UnitId::new(0),
    );
    assert_eq!(s.reg_word(5), 1);
}
