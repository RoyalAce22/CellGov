//! Each facility's `rchcnt` count moves with its state.

use super::*;
use crate::exec::SpuFault;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu;

fn uid() -> UnitId {
    UnitId::new(0)
}

fn count(s: &mut SpuState, channel: u8) -> u32 {
    let out = execute(&SpuInstruction::Rchcnt { rt: 3, channel }, s, uid());
    assert_eq!(out, SpuStepOutcome::Continue, "rchcnt {channel}");
    assert_eq!(
        s.regs[3][4..],
        [0; 12],
        "rchcnt {channel} zeroes the other slots"
    );
    s.reg_word(3)
}

#[test]
fn a_channel_the_model_does_not_implement_refuses_its_count() {
    let mut s = SpuState::new();
    assert_eq!(
        execute(&SpuInstruction::Rchcnt { rt: 3, channel: 7 }, &mut s, uid()),
        SpuStepOutcome::Fault(SpuFault::UnsupportedChannelCount(7))
    );
}

/// [CBEA p:128 s:9.3.6] MFC_RdTagStat counts 1 once the requested status is available, and 0 again after it is read.
#[test]
fn the_tag_status_count_follows_a_request_and_its_read() {
    let mut s = SpuState::new();
    s.channels.tag_mask = 0b10;
    s.set_reg_word_splat(4, spu::MFC_TAG_UPDATE_ALL);
    let wrch = |channel| SpuInstruction::Wrch { channel, rt: 4 };
    execute(&wrch(spu::MFC_WR_TAG_UPDATE), &mut s, uid());
    assert_eq!(
        count(&mut s, spu::MFC_RD_TAG_STAT),
        0,
        "tag 1 is still in flight"
    );
    // The unit rebuilds the status and settles the request at step entry.
    s.channels.tag_status = 0b10;
    s.channels.settle_tag_update();
    assert_eq!(count(&mut s, spu::MFC_RD_TAG_STAT), 1);
    execute(
        &SpuInstruction::Rdch {
            rt: 5,
            channel: spu::MFC_RD_TAG_STAT,
        },
        &mut s,
        uid(),
    );
    assert_eq!(count(&mut s, spu::MFC_RD_TAG_STAT), 0);
}

/// [CBEA p:131 s:9.4] MFC_RdAtomicStat counts 1 once an atomic command completes, and a read consumes it.
#[test]
fn the_atomic_status_count_follows_a_putllc_and_its_read() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(4, u32::from(spu::MFC_PUTLLC as u8));
    execute(
        &SpuInstruction::Wrch {
            channel: spu::MFC_CMD,
            rt: 4,
        },
        &mut s,
        uid(),
    );
    assert_eq!(count(&mut s, spu::MFC_RD_ATOMIC_STAT), 1);
    execute(
        &SpuInstruction::Rdch {
            rt: 5,
            channel: spu::MFC_RD_ATOMIC_STAT,
        },
        &mut s,
        uid(),
    );
    assert_eq!(count(&mut s, spu::MFC_RD_ATOMIC_STAT), 0);
}

/// [CBEA p:131 s:9.4] the channel is read-blocking with a maximum count of 1, and a read clears its contents.
#[test]
fn a_second_atomic_status_read_stalls_and_leaves_its_register() {
    let mut s = SpuState::new();
    // A putllc with no reservation fails, and reports S.
    s.set_reg_word_splat(4, spu::MFC_PUTLLC);
    execute(
        &SpuInstruction::Wrch {
            channel: spu::MFC_CMD,
            rt: 4,
        },
        &mut s,
        uid(),
    );
    let rdch = SpuInstruction::Rdch {
        rt: 5,
        channel: spu::MFC_RD_ATOMIC_STAT,
    };
    assert_eq!(execute(&rdch, &mut s, uid()), SpuStepOutcome::Continue);
    assert_eq!(s.reg_word(5), spu::MFC_ATOMIC_STAT_S);
    assert_eq!(s.channels.atomic_status, 0, "the read cleared the status");

    s.set_reg_word_splat(5, 0xDEAD_BEEF);
    assert!(
        matches!(
            execute(&rdch, &mut s, uid()),
            SpuStepOutcome::Yield {
                reason: cellgov_exec::YieldReason::ChannelStall,
                ..
            }
        ),
        "no command completed since the read"
    );
    assert_eq!(s.reg_word(5), 0xDEAD_BEEF);
}

/// [CBEA p:131 s:9.4] a successful putllc and a getllar are immediate atomic commands too, so each makes the count 1.
#[test]
fn a_successful_putllc_and_a_getllar_each_make_the_atomic_status_count_1() {
    let mut s = SpuState::new();
    s.reservation = Some(cellgov_sync::ReservedLine::containing(0x80));
    s.channels.mfc_eal = 0x80;
    s.set_reg_word_splat(4, spu::MFC_PUTLLC);
    let wrch_cmd = SpuInstruction::Wrch {
        channel: spu::MFC_CMD,
        rt: 4,
    };
    let out = execute(&wrch_cmd, &mut s, uid());
    assert!(
        matches!(out, SpuStepOutcome::Yield { .. }),
        "the putllc holds its reservation, so it stores: {out:?}"
    );
    assert_eq!(count(&mut s, spu::MFC_RD_ATOMIC_STAT), 1);

    use cellgov_exec::{ExecutionContext, ExecutionUnit};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    /// `wrch MFC_Cmd, r4`, `rchcnt r3, MFC_RdAtomicStat`, then `stop`.
    const PROGRAM: [u32; 3] = [
        (0x10D << 21) | ((spu::MFC_CMD as u32) << 7) | 4,
        (0x00F << 21) | ((spu::MFC_RD_ATOMIC_STAT as u32) << 7) | 3,
        0,
    ];
    let mut unit = crate::SpuExecutionUnit::new(UnitId::new(1));
    for (i, word) in PROGRAM.iter().enumerate() {
        unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    unit.state_mut().channels.mfc_lsa = 0x100;
    unit.state_mut().channels.mfc_eal = 0x80;
    unit.state_mut().set_reg_word_splat(4, spu::MFC_GETLLAR);
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    unit.run_until_yield(Budget::new(10), &ExecutionContext::new(&mem), &mut effects);
    assert!(unit.state().reservation.is_some(), "the getllar completed");
    assert_eq!(unit.state().reg_word(3), 1);
}

/// [CBEA p:135 s:9.5.3] SPU_RdInMbox counts the messages waiting; [CBE-Handbook p:445 s:17.1 Table 17-2] it holds at most 4.
#[test]
fn the_inbound_mailbox_count_is_the_runtime_occupancy_up_to_the_depth() {
    use cellgov_exec::{ExecutionContext, ExecutionUnit};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    /// `rchcnt r3, SPU_RdInMbox` then `stop`.
    const PROGRAM: [u32; 2] = [(0x00F << 21) | ((spu::SPU_RD_IN_MBOX as u32) << 7) | 3, 0];
    for (occupancy, want) in [(0, 0), (2, 2), (9, 4)] {
        let mut unit = crate::SpuExecutionUnit::new(UnitId::new(1));
        for (i, word) in PROGRAM.iter().enumerate() {
            unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let mem = GuestMemory::new(0x1000);
        let messages: Vec<u32> = (0..occupancy).collect();
        let ctx = ExecutionContext::new(&mem).with_inbound_mailbox(&messages);
        let mut effects = Vec::new();
        unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
        assert_eq!(unit.state().reg_word(3), want, "occupancy {occupancy}");
    }
}

fn write_channel(s: &mut SpuState, channel: u8, value: u32) -> SpuStepOutcome {
    s.set_reg_word_splat(4, value);
    execute(&SpuInstruction::Wrch { channel, rt: 4 }, s, uid())
}

fn read_channel(s: &mut SpuState, channel: u8) -> SpuStepOutcome {
    execute(&SpuInstruction::Rdch { rt: 5, channel }, s, uid())
}

/// [CBEA p:151 s:9.11.2] the pending register records every event whatever the mask, and enabling a pending event updates SPU_RdEventStat.
/// [CBEA p:147 s:9.11.1] a read returns the enabled pending events and sets the count to 0; a read with count 0 stalls.
#[test]
fn a_masked_event_waits_pending_until_the_mask_enables_it() {
    use spu::event::{MB, S1};
    let mut s = SpuState::new();
    s.raise_events(S1 | MB);
    assert_eq!(count(&mut s, spu::SPU_RD_EVENT_STAT), 0, "neither enabled");

    write_channel(&mut s, spu::SPU_WR_EVENT_MASK, S1);
    assert_eq!(count(&mut s, spu::SPU_RD_EVENT_STAT), 1);
    assert_eq!(
        read_channel(&mut s, spu::SPU_RD_EVENT_STAT),
        SpuStepOutcome::Continue
    );
    assert_eq!(s.reg_word(5), S1, "the masked Mb is not reported");
    assert_eq!(count(&mut s, spu::SPU_RD_EVENT_STAT), 0);
    assert!(
        matches!(
            read_channel(&mut s, spu::SPU_RD_EVENT_STAT),
            SpuStepOutcome::Yield {
                reason: cellgov_exec::YieldReason::ChannelStall,
                ..
            }
        ),
        "a read with count 0 waits for an event"
    );
}

/// [CBEA p:155 s:9.11.4] an acknowledgment resets its events, enabled or not.
/// [CBEA p:147 s:9.11.1] enabled events still pending after an acknowledgment set the count to 1.
#[test]
fn an_acknowledgment_that_leaves_an_enabled_event_pending_sets_the_count() {
    use spu::event::{MB, S1, S2};
    let mut s = SpuState::new();
    write_channel(&mut s, spu::SPU_WR_EVENT_MASK, S1 | S2);
    s.raise_events(S1 | S2 | MB);
    read_channel(&mut s, spu::SPU_RD_EVENT_STAT);
    assert_eq!(s.reg_word(5), S1 | S2);

    write_channel(&mut s, spu::SPU_WR_EVENT_ACK, S1 | MB);
    assert_eq!(s.channels.pending_events, S2, "the disabled Mb clears too");
    assert_eq!(
        count(&mut s, spu::SPU_RD_EVENT_STAT),
        1,
        "S2 is still pending"
    );
    read_channel(&mut s, spu::SPU_RD_EVENT_STAT);
    assert_eq!(s.reg_word(5), S2);

    write_channel(&mut s, spu::SPU_WR_EVENT_ACK, S2);
    assert_eq!(count(&mut s, spu::SPU_RD_EVENT_STAT), 0);
}

/// [CBEA p:153 s:9.11.3] SPU_RdEventMask returns the last data written to SPU_WrEventMask.
#[test]
fn the_event_mask_reads_back_as_written() {
    let mut s = SpuState::new();
    write_channel(&mut s, spu::SPU_WR_EVENT_MASK, 0x8000_1234);
    read_channel(&mut s, spu::SPU_RD_EVENT_MASK);
    assert_eq!(s.reg_word(5), 0x8000_1234);
}

/// [CBEA p:148 s:9.11.1] a MFC_WrMSSyncReq write with no transfer pending triggers Ms at once.
#[test]
fn a_multisource_request_with_nothing_in_flight_raises_ms_at_once() {
    let mut s = SpuState::new();
    write_channel(&mut s, spu::SPU_WR_EVENT_MASK, spu::event::MS);
    write_channel(&mut s, spu::MFC_WR_MSSYNC_REQ, 0);
    assert_eq!(count(&mut s, spu::SPU_RD_EVENT_STAT), 1);
    read_channel(&mut s, spu::SPU_RD_EVENT_STAT);
    assert_eq!(s.reg_word(5), spu::event::MS);
}

/// [CBEA p:149 s:9.11.1] an event is the edge of its source's count from 0 to nonzero, so a count that stays up raises it once.
#[test]
fn a_source_count_raises_its_event_on_the_rising_edge_alone() {
    let mut s = SpuState::new();
    s.update_events();
    assert_eq!(
        s.channels.pending_events, 0,
        "the Ms count starts at 1 with no edge"
    );
    s.channels.in_mbox = vec![7];
    s.update_events();
    assert_eq!(s.channels.pending_events, spu::event::MB);
    s.channels.pending_events = 0;
    s.update_events();
    assert_eq!(s.channels.pending_events, 0, "no new edge");
    s.channels.in_mbox.clear();
    s.update_events();
    s.channels.in_mbox = vec![8];
    s.update_events();
    assert_eq!(s.channels.pending_events, spu::event::MB, "a new edge");
}
