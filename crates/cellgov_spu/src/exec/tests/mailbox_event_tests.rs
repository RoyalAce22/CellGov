//! The Le and Me events follow the outbound mailbox counts, as the
//! `spu_mailbox_events` console capture shows them.

use super::*;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu::{self, event};

fn uid() -> UnitId {
    UnitId::new(0)
}

fn wrch(s: &mut SpuState, channel: u8, value: u32) {
    s.set_reg_word_splat(4, value);
    let out = execute(&SpuInstruction::Wrch { channel, rt: 4 }, s, uid());
    assert_eq!(out, SpuStepOutcome::Continue, "wrch {channel}");
}

fn event_count(s: &mut SpuState) -> u32 {
    let out = execute(
        &SpuInstruction::Rchcnt {
            rt: 3,
            channel: spu::SPU_RD_EVENT_STAT,
        },
        s,
        uid(),
    );
    assert_eq!(out, SpuStepOutcome::Continue);
    s.reg_word(3)
}

/// The probe's `take_status`: the status read and acknowledged when the
/// count is not 0, else 0.
fn take_status(s: &mut SpuState) -> u32 {
    if event_count(s) == 0 {
        return 0;
    }
    let out = execute(
        &SpuInstruction::Rdch {
            rt: 5,
            channel: spu::SPU_RD_EVENT_STAT,
        },
        s,
        uid(),
    );
    assert_eq!(out, SpuStepOutcome::Continue);
    let status = s.reg_word(5);
    wrch(s, spu::SPU_WR_EVENT_ACK, status);
    status
}

/// The probe's `quiet()`: every event masked and acknowledged.
fn quiet(s: &mut SpuState) {
    wrch(s, spu::SPU_WR_EVENT_MASK, 0);
    wrch(s, spu::SPU_WR_EVENT_ACK, u32::MAX);
}

/// A started, quiet SPU whose interrupt mailbox LV2 drains.
fn started() -> SpuState {
    let mut s = SpuState::new();
    s.channels.out_intr_mbox_connected = true;
    s.update_events();
    quiet(&mut s);
    s
}

#[test]
fn a_new_spu_has_le_pending_and_neither_me_nor_a() {
    let mut s = SpuState::new();
    s.channels.out_intr_mbox_connected = true;
    s.update_events();
    wrch(&mut s, spu::SPU_WR_EVENT_MASK, event::A);
    assert_eq!(event_count(&mut s), 0, "A");
    wrch(&mut s, spu::SPU_WR_EVENT_MASK, event::LE | event::ME);
    assert_eq!(take_status(&mut s), event::LE);
}

#[test]
fn a_full_outbound_mailbox_raises_le_once_it_is_read() {
    let mut s = started();
    wrch(&mut s, spu::SPU_WR_EVENT_MASK, event::LE | event::ME);
    wrch(&mut s, spu::SPU_WR_OUT_MBOX, 0x1111_1111);
    s.update_events();
    assert_eq!(take_status(&mut s), 0, "full: nothing yet");
    assert_eq!(s.channels.out_mbox.take(), Some(0x1111_1111));
    s.update_events();
    assert_eq!(take_status(&mut s), event::LE);
}

#[test]
fn a_full_interrupt_mailbox_raises_me_once_it_is_read() {
    let mut s = started();
    wrch(&mut s, spu::SPU_WR_EVENT_MASK, event::LE | event::ME);
    wrch(&mut s, spu::SPU_WR_OUT_INTR_MBOX, 0x0122_2222);
    s.update_events();
    assert_eq!(take_status(&mut s), 0, "full: nothing yet");
    assert_eq!(s.channels.out_intr_mbox.take(), Some(0x0122_2222));
    s.update_events();
    assert_eq!(take_status(&mut s), event::ME);
}

#[test]
fn le_raised_while_masked_stays_pending_until_enabled() {
    let mut s = started();
    wrch(&mut s, spu::SPU_WR_EVENT_MASK, event::ME);
    wrch(&mut s, spu::SPU_WR_OUT_MBOX, 0x3333_3333);
    s.update_events();
    s.channels.out_mbox = None;
    s.update_events();
    assert_eq!(event_count(&mut s), 0, "Le is masked");
    wrch(&mut s, spu::SPU_WR_EVENT_MASK, event::LE | event::ME);
    assert_eq!(take_status(&mut s), event::LE);
}
