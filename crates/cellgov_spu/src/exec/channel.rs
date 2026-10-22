//! The channel reads and writes and the MFC command path.

use crate::state::SpuState;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::ChannelDirection;

use super::mfc::{acknowledge_list_stall, execute_mfc_cmd};
use super::outcome::{SpuFault, SpuStepOutcome};

/// The yield of a blocking channel access whose count is zero. The
/// access does not retire; the unit parks on it and runs it again when
/// woken.
///
/// [CBEA p:109 s:9] a blocking channel access completes only when the channel count is non-zero; otherwise the SPU stalls.
/// [SPU-ISA p:257 s:13.7] channel accesses are never reordered or speculated, so the stalled access is the next one to run.
pub(super) fn stall() -> SpuStepOutcome {
    SpuStepOutcome::Yield {
        effects: vec![],
        reason: YieldReason::ChannelStall,
    }
}

pub(super) fn execute_wrch(
    channel: u8,
    rt: u8,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    // [CBE-Handbook p:442 s:17.1] the CBE differs from the SPU ISA: a wrch to a valid read channel has no effect.
    // The `spu_invalid_channel` console capture: a wrch to SPU_RdInMbox does not stop the SPU.
    if spu::channel_direction(channel) == Some(ChannelDirection::Read) {
        return SpuStepOutcome::Continue;
    }
    let val = state.reg_word(rt);
    match channel {
        // [CBE-Handbook p:453 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_LSA stores the local-store address for the MFC command being formed.
        spu::MFC_LSA => {
            state.channels.mfc_lsa = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:454 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_EAH holds the high 32 bits of the 64-bit effective address.
        spu::MFC_EAH => {
            state.channels.mfc_eah = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:455 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_EAL holds the low 32 bits of the effective address; alignment depends on transfer size.
        spu::MFC_EAL => {
            state.channels.mfc_eal = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:455 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_Size sets the transfer size in bytes (max 16 KB).
        spu::MFC_SIZE => {
            state.channels.mfc_size = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:456 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] MFC_TagID assigns a 0..31 tag value to the command being formed.
        // [CBEA p:115 s:9. Synergistic Processor Unit Channels sub:9.1 MFC SPU Command Parameter Channels] The parameter's validity is checked asynchronous to the instruction stream, so the write itself stands whatever the guest wrote; `execute_mfc_cmd` gates the command that would carry it.
        spu::MFC_TAG_ID => {
            state.channels.mfc_tag_id = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:457 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] Writing the Class ID and MFC Command Opcode enqueues the command into the SPU MFC command queue.
        spu::MFC_CMD => {
            let outcome = execute_mfc_cmd(val, state, unit_id);
            // A command that did not write MFC_EAH names the high word 0,
            // so the MFC takes EAH back to 0 once a command has used it.
            // A write that stalls on a full queue used nothing, and so would
            // a command the model does not run; no defined opcode is one.
            // [CBEA p:52 s:7] when EAH is not specified on a command, hardware must set EAH to '0'.
            // [CBEA p:121 s:9.2] footnote 1: EAH is optional and is set to zero if not written.
            if !matches!(
                outcome,
                SpuStepOutcome::Yield {
                    reason: YieldReason::ChannelStall,
                    ..
                } | SpuStepOutcome::Fault(_)
            ) {
                state.channels.mfc_eah = 0;
            }
            outcome
        }
        // [CBE-Handbook p:458 s:17. SPE Channel and Related MMIO Interface sub:17.10 MFC Tag-Group Management Channels] MFC_WrTagMask selects the tag groups included in subsequent tag-status queries.
        spu::MFC_WR_TAG_MASK => {
            state.channels.tag_mask = val;
            SpuStepOutcome::Continue
        }
        // [CBEA p:127 s:9.3.5] MFC_WrTagUpdate sets when the tag status updates: immediately, when any enabled group completes, or when all do.
        // [CBE-Handbook p:459 s:17.10] bits 0:29 are reserved, and TS 11 is a reserved update condition.
        // The `spu_mfc_command_edges` console capture: TS 11 requests no
        // update and raises no error, so the MFC_RdTagStat count stays 0.
        // The model refuses a write with a reserved bit set by name.
        spu::MFC_WR_TAG_UPDATE => {
            if val > spu::MFC_TAG_UPDATE_RESERVED {
                return SpuStepOutcome::Fault(SpuFault::ReservedTagUpdate(val));
            }
            state.channels.request_tag_update(val);
            SpuStepOutcome::Continue
        }
        // [CBEA p:130 s:9.3.8] MFC_WrListStallAck names a tag group in bits 27:31 and restarts that group's stalled list commands.
        spu::MFC_WR_LIST_STALL_ACK => acknowledge_list_stall(val, state, unit_id),
        // [CBE-Handbook p:463 s:17. SPE Channel and Related MMIO Interface sub:17.12 SPU Mailbox Channels] SPU Write Outbound Mailbox sends a 32-bit message to the PPE.
        // [CBEA p:98 s:8.6.1] a write to a full outbound mailbox stalls the SPU until another processor reads it.
        spu::SPU_WR_OUT_MBOX => {
            if state.channels.out_mbox.is_some() {
                return stall();
            }
            state.channels.out_mbox = Some(val);
            SpuStepOutcome::Continue
        }
        // [CBEA p:134 s:9.5.2] a write to a full outbound interrupt mailbox stalls the SPU until another processor reads it.
        // A unit whose interrupt mailbox no reader empties refuses the
        // channel by name below.
        spu::SPU_WR_OUT_INTR_MBOX if state.channels.out_intr_mbox_connected => {
            if state.channels.out_intr_mbox.is_some() {
                return stall();
            }
            state.channels.out_intr_mbox = Some(val);
            state.channels.out_intr_mbox_interrupt = true;
            SpuStepOutcome::Continue
        }
        // [CBEA p:151 s:9.11.2] the mask selects the pending events SPU_RdEventStat reports; enabling a pending event updates the channel.
        // [CBEA p:153 s:9.11.3] SPU_RdEventMask returns the last data written, so the model keeps it whole.
        spu::SPU_WR_EVENT_MASK => {
            let pending = state.channels.pending_events;
            state.channels.set_event_state(pending, val);
            SpuStepOutcome::Continue
        }
        // [CBEA p:155 s:9.11.4] a 1 bit resets that pending event, enabled or not.
        // [CBEA p:146 s:9.11] the count also increments if an event is still set in the status after the write.
        spu::SPU_WR_EVENT_ACK => {
            // A crossing before the write is an event the write can
            // acknowledge.
            state.sample_decrementer();
            let now = state.time_base();
            let c = &mut state.channels;
            c.set_event_state(c.pending_events & !val, c.event_mask);
            if c.pending_events & c.event_mask != 0 {
                c.event_count = true;
            }
            // [CBEA p:152 s:9.11.2] acknowledging Tm while it is disabled stops the decrementer; while it is enabled the decrementer runs on.
            // The spu_events capture: after an acknowledgment of every
            // event with the mask clear, the count holds across a spin.
            if val & spu::event::TM != 0 && c.event_mask & spu::event::TM == 0 {
                c.decrementer.stop(now);
            }
            SpuStepOutcome::Continue
        }
        // [CBEA p:143 s:9.10] a write starts tracking the transfers outstanding to the MFC, and a second write stalls until those complete; the data written is ignored.
        // [CBEA p:104 s:8.8] the tracked transfers are those to or from the associated MFC received before the request.
        // A command the unit issues ends its step, so the queue the step
        // began with holds every transfer received before this write.
        spu::MFC_WR_MSSYNC_REQ => {
            if state.channels.mssync_tracking.is_some() {
                return stall();
            }
            state.channels.mssync_tracking = state.channels.mssync_horizon;
            // [CBEA p:148 s:9.11.1] Ms is triggered at once when no transfer is pending at the write.
            if state.channels.mssync_tracking.is_none() {
                state.raise_events(spu::event::MS);
            }
            SpuStepOutcome::Continue
        }
        // [CBEA p:142 s:9.9.1] a write to SPU_WrSRR0 updates SRR0.
        // The architecture asks for a sync.c before an instruction that
        // depends on the new value, and calls SRR0 indeterminate after a
        // write with interrupts enabled. The model takes the write at
        // once in both cases, which is CellGov's choice.
        spu::SPU_WR_SRR0 => {
            state.set_srr0(val);
            SpuStepOutcome::Continue
        }
        // [CBEA p:139 s:9.7.1] SPU_WrDec loads the decrementer and starts it.
        spu::SPU_WR_DEC => {
            let now = state.time_base();
            if state.channels.decrementer.write(now, val) {
                state.raise_events(spu::event::TM);
            }
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:443 s:17.1.4] a write to a reserved channel has no effect and raises no interrupt.
        _ if spu::is_reserved_channel(channel) => SpuStepOutcome::Continue,
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedChannel {
            channel,
            is_write: true,
        }),
    }
}

pub(super) fn execute_rdch(
    rt: u8,
    channel: u8,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    // [CBE-Handbook p:442 s:17.1] the CBE differs from the SPU ISA: an rdch of a valid write channel returns zeros.
    // The `spu_invalid_channel` console capture: an rdch of SPU_WrOutMbox reads 0 and does not stop the SPU.
    if spu::channel_direction(channel) == Some(ChannelDirection::Write) {
        state.set_reg_channel_word(rt, 0);
        return SpuStepOutcome::Continue;
    }
    match channel {
        // [CBE-Handbook p:460 s:17.10.4] MFC_RdTagStat reports the status from the last tag-group status update request.
        // [CBEA p:127 s:9.3.5] a read with no update request is a software-induced deadlock.
        // The model does not park the SPU on that deadlock and refuses the
        // read by name, unless an interrupt can end the wait.
        spu::MFC_RD_TAG_STAT => match state.channels.tag_status_read.take() {
            Some(status) => {
                state.set_reg_channel_word(rt, status);
                SpuStepOutcome::Continue
            }
            None if state.channels.tag_update.is_some() || state.interruptible() => stall(),
            None => SpuStepOutcome::Fault(SpuFault::ChannelStall(channel)),
        },
        // [CBEA p:129 s:9.3.7] MFC_RdListStallStat returns the tag groups whose list stalled since the last read, clears them, and blocks while there are none.
        // A read never completes when the status is 0 and no list has a
        // stall still to come. The model refuses that read by name, unless
        // an interrupt can end the wait.
        spu::MFC_RD_LIST_STALL_STAT => {
            let status = core::mem::take(&mut state.channels.list_stall_status);
            if status != 0 {
                state.set_reg_channel_word(rt, status);
                SpuStepOutcome::Continue
            } else if state.channels.lists.iter().any(|list| !list.stalled) || state.interruptible()
            {
                stall()
            } else {
                SpuStepOutcome::Fault(SpuFault::ChannelStall(channel))
            }
        }
        // [CBEA p:126 s:9.3.4] MFC_RdTagMask returns the current tag-group query mask.
        spu::MFC_RD_TAG_MASK => {
            state.set_reg_channel_word(rt, state.channels.tag_mask);
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:543 s:19. DMA Transfers and Interprocessor Communication sub:19.6 Mailboxes] SPU Read Inbound Mailbox is read-blocking when the mailbox is empty.
        // The read takes the oldest message and the commit removes it
        // from the mailbox; the step then yields so the next step sees
        // the mailbox without it.
        // [CBEA p:147 s:9.11.1] a read with count 0 stalls; a read with count 1 returns the enabled pending events and sets the count to 0.
        spu::SPU_RD_EVENT_STAT => {
            if !state.channels.event_count {
                return stall();
            }
            let c = &mut state.channels;
            c.event_count = false;
            let status = c.pending_events & c.event_mask;
            state.set_reg_channel_word(rt, status);
            SpuStepOutcome::Continue
        }
        // [CBEA p:153 s:9.11.3] a read returns the last data written to SPU_WrEventMask.
        spu::SPU_RD_EVENT_MASK => {
            state.set_reg_channel_word(rt, state.channels.event_mask);
            SpuStepOutcome::Continue
        }
        spu::SPU_RD_IN_MBOX => {
            if state.channels.in_mbox.is_empty() {
                return stall();
            }
            let message = state.channels.in_mbox.remove(0);
            state.set_reg_channel_word(rt, message);
            SpuStepOutcome::Yield {
                effects: vec![Effect::MailboxPop {
                    mailbox: cellgov_sync::MailboxId::new(unit_id.raw()),
                    message: cellgov_effects::MailboxMessage::new(message),
                    source: unit_id,
                }],
                reason: YieldReason::MailboxAccess,
            }
        }
        // [CBEA p:136 s:9.6] a read with count 1 returns the contents and resets the contents and the count to 0; a read with count 0 stalls.
        // [CBEA p:137 s:9.6.1], [CBEA p:138 s:9.6.2] the read resets the bits that were set, in either mode.
        // A write from another processor lands between steps, so a read
        // never meets a write that is part done.
        spu::SPU_RD_SIG_NOTIFY_1 | spu::SPU_RD_SIG_NOTIFY_2 => {
            let signal = &mut state.signals[usize::from(channel == spu::SPU_RD_SIG_NOTIFY_2)];
            if !signal.pending {
                return stall();
            }
            let word = signal.word;
            signal.word = 0;
            signal.pending = false;
            state.set_reg_channel_word(rt, word);
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:462 s:17. SPE Channel and Related MMIO Interface sub:17.11 MFC Read Atomic Command Status Channel] Reports success/failure status for the most recent atomic command (e.g. putllc).
        // [CBEA p:131 s:9.4] the channel is read-blocking with a maximum count of 1, and a read clears its contents.
        spu::MFC_RD_ATOMIC_STAT => {
            if !state.channels.atomic_status_ready {
                return stall();
            }
            state.set_reg_channel_word(rt, state.channels.atomic_status);
            state.channels.atomic_status = 0;
            state.channels.atomic_status_ready = false;
            SpuStepOutcome::Continue
        }
        // [CBEA p:141 s:9.8 SPU Read Machine Status Channel] Two status bits: IS (bit 30) isolation and IE (bit 31) interrupt enable; the model runs nonisolated, so IS reads as zero.
        // The isolation facility is out of scope; docs/architecture/execution_units.md records why.
        spu::SPU_RD_MACH_STAT => {
            state.set_reg_channel_word(rt, u32::from(state.interrupts_enabled()));
            SpuStepOutcome::Continue
        }
        // [CBEA p:142 s:9.9.2] a read of SPU_RdSRR0 returns the contents of SRR0.
        spu::SPU_RD_SRR0 => {
            state.set_reg_channel_word(rt, state.srr0());
            SpuStepOutcome::Continue
        }
        // [CBEA p:140 s:9.7.2] SPU_RdDec returns the current count.
        spu::SPU_RD_DEC => {
            let now = state.time_base();
            let (value, crossed) = state.channels.decrementer.read(now);
            if crossed {
                state.raise_events(spu::event::TM);
            }
            state.set_reg_channel_word(rt, value);
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:443 s:17.1.4] a read of a reserved channel returns zeros and raises no interrupt.
        _ if spu::is_reserved_channel(channel) => {
            state.set_reg_channel_word(rt, 0);
            SpuStepOutcome::Continue
        }
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedChannel {
            channel,
            is_write: false,
        }),
    }
}

pub(super) fn execute_rchcnt(rt: u8, channel: u8, state: &mut SpuState) -> SpuStepOutcome {
    let Some(count) = channel_count(channel, state) else {
        return SpuStepOutcome::Fault(SpuFault::UnsupportedChannelCount(channel));
    };
    // [SPU-ISA p:249 s:11] rchcnt places the channel count in the preferred slot and zeros bytes 4 to 15.
    state.set_reg_channel_word(rt, count);
    SpuStepOutcome::Continue
}

/// The count `rchcnt` reads for `channel`, or `None` for a channel the
/// model does not implement.
///
/// A nonblocking channel counts 1. A blocking channel counts its free
/// capacity (a write channel) or its occupancy (a read channel). Where
/// the model keeps no queue for a channel, the count is the one that
/// model implies, as each arm states.
///
/// [CBEA p:109 s:9] a nonblocking channel's rchcnt returns 1; a blocking channel's count is its free capacity or occupancy.
pub(super) fn channel_count(channel: u8, state: &SpuState) -> Option<u32> {
    let channels = &state.channels;
    Some(match channel {
        // [CBEA p:141 s:9.8 SPU Read Machine Status Channel] The channel has no count; rchcnt on it always returns 1.
        spu::SPU_RD_MACH_STAT => 1,
        // [CBEA p:142 s:9.9.1], [CBEA p:142 s:9.9.2] the SRR0 channels have no count; rchcnt on either returns 1.
        spu::SPU_WR_SRR0 | spu::SPU_RD_SRR0 => 1,
        // [CBEA p:140 s:9.7.2] the decrementer channels have no count; rchcnt on either returns 1.
        spu::SPU_WR_DEC | spu::SPU_RD_DEC => 1,
        // [CBEA p:112 s:9.1] the MFC command parameter channels are nonblocking and count 1.
        spu::MFC_LSA | spu::MFC_EAH | spu::MFC_EAL | spu::MFC_SIZE | spu::MFC_TAG_ID => 1,
        // [CBEA p:113 s:9.1.1] MFC_Cmd counts the free command-queue slots.
        spu::MFC_CMD => channels.cmd_queue_free,
        // [CBEA p:125 s:9.3.3] MFC_WrTagMask is nonblocking and has no count.
        spu::MFC_WR_TAG_MASK => 1,
        // [CBEA p:127 s:9.3.5] MFC_WrTagUpdate counts 0 until the MFC takes the request, then 1.
        // The model takes each request as it is written.
        spu::MFC_WR_TAG_UPDATE => 1,
        // [CBEA p:128 s:9.3.6] MFC_RdTagStat counts 1 once the requested tag status is available.
        spu::MFC_RD_TAG_STAT => u32::from(channels.tag_status_read.is_some()),
        // [CBEA p:126 s:9.3.4] MFC_RdTagMask is nonblocking and counts 1.
        spu::MFC_RD_TAG_MASK => 1,
        // [CBEA p:131 s:9.4] MFC_RdAtomicStat counts 1 once an immediate atomic command completes.
        spu::MFC_RD_ATOMIC_STAT => u32::from(channels.atomic_status_ready),
        // [CBEA p:133 s:9.5.1] SPU_WrOutMbox counts its free entries.
        spu::SPU_WR_OUT_MBOX => spu::SPU_OUT_MBOX_DEPTH - u32::from(channels.out_mbox.is_some()),
        // [CBEA p:143 s:9.10] the channel counts 1 until a write, and returns to 1 when the synchronization completes.
        spu::MFC_WR_MSSYNC_REQ => u32::from(channels.mssync_tracking.is_none()),
        // [CBEA p:135 s:9.5.3] SPU_RdInMbox counts the messages in the inbound mailbox.
        spu::SPU_RD_IN_MBOX => u32::try_from(channels.in_mbox.len())
            .unwrap_or(u32::MAX)
            .min(spu::SPU_IN_MBOX_DEPTH),
        // [CBEA p:147 s:9.11.1] SPU_RdEventStat counts 1 once an enabled event is pending.
        spu::SPU_RD_EVENT_STAT => u32::from(channels.event_count),
        // [CBEA p:151 s:9.11.2], [CBEA p:153 s:9.11.3], [CBEA p:155 s:9.11.4] the mask and acknowledgment channels always count 1.
        spu::SPU_WR_EVENT_MASK | spu::SPU_RD_EVENT_MASK | spu::SPU_WR_EVENT_ACK => 1,
        // [CBEA p:137 s:9.6.1], [CBEA p:138 s:9.6.2] a signal-notification channel counts 1 while unread signals are pending.
        spu::SPU_RD_SIG_NOTIFY_1 => u32::from(state.signals[0].pending),
        spu::SPU_RD_SIG_NOTIFY_2 => u32::from(state.signals[1].pending),
        // [CBEA p:129 s:9.3.7] MFC_RdListStallStat counts 1 once a list element with the stall-and-notify flag completes.
        spu::MFC_RD_LIST_STALL_STAT => u32::from(channels.list_stall_status != 0),
        // [CBEA p:130 s:9.3.8] MFC_WrListStallAck is nonblocking and counts 1.
        spu::MFC_WR_LIST_STALL_ACK => 1,
        // [CBEA p:134 s:9.5.2] SPU_WrOutIntrMbox counts its free entries.
        // A unit that refuses the write keeps the one entry free.
        spu::SPU_WR_OUT_INTR_MBOX => {
            spu::SPU_OUT_INTR_MBOX_DEPTH - u32::from(channels.out_intr_mbox.is_some())
        }
        // The reference console counts three reserved channels as 1.
        _ if spu::SPU_RESERVED_CHANNELS_COUNTING_ONE.contains(&channel) => 1,
        // [CBE-Handbook p:443 s:17.1.3] rchcnt on a reserved channel returns 0.
        _ if spu::is_reserved_channel(channel) => 0,
        _ => return None,
    })
}
