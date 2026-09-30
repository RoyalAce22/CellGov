//! The channel reads and writes and the MFC command path.

use crate::state::{ListCursor, SpuState};
use crate::stop::SpuStopKind;
use cellgov_dma::DmaDirection::{Get, Put};
use cellgov_dma::MfcCommandClass::{SendSignal, Transfer};
use cellgov_dma::MfcOrdering as Order;
use cellgov_dma::{
    DmaDirection, DmaRequest, InvalidMfcCommand, MfcCommandClass, MfcCommandError, MfcOrdering,
    MfcParameters,
};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{BarrierKind, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::{ChannelDirection, MfcCmd, MfcTagId, MFC_ATOMIC_STAT_S};
use cellgov_ps3_abi::hw::spu_mfc::{MfcOpcodeClass, MfcQueues};
use cellgov_sync::RESERVATION_LINE_BYTES;
use cellgov_time::GuestTicks;

use super::outcome::{SpuFault, SpuStepOutcome};

/// The yield of a blocking channel access whose count is zero. The
/// access does not retire; the unit parks on it and runs it again when
/// woken.
///
/// [CBEA p:109 s:9] a blocking channel access completes only when the channel count is non-zero; otherwise the SPU stalls.
/// [SPU-ISA p:257 s:13.7] channel accesses are never reordered or speculated, so the stalled access is the next one to run.
fn stall() -> SpuStepOutcome {
    SpuStepOutcome::Yield {
        effects: vec![],
        reason: YieldReason::ChannelStall,
    }
}

/// The stop a channel instruction in the wrong direction takes.
fn invalid_channel() -> SpuStepOutcome {
    SpuStepOutcome::Stop {
        kind: SpuStopKind::InvalidChannel,
        signal: 0,
    }
}

pub(super) fn execute_wrch(
    channel: u8,
    rt: u8,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    // [CBEA p:109 s:9] a channel instruction the channel's definition does not allow raises an invalid channel instruction interrupt.
    // [CBEA p:93 s:8.5.2] SPU_Status[C]: an invalid channel instruction was detected and the SPU stopped.
    if spu::channel_direction(channel) == Some(ChannelDirection::Read) {
        return invalid_channel();
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
            // A write that stalls on a full queue used nothing, and so did
            // a command the model does not run.
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
        // The model gives a reserved request no meaning and refuses it by
        // name.
        spu::MFC_WR_TAG_UPDATE => {
            if val > spu::MFC_TAG_UPDATE_ALL {
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
    // [CBEA p:109 s:9] an rdch to a write or write-blocking channel raises an invalid channel instruction interrupt.
    if spu::channel_direction(channel) == Some(ChannelDirection::Write) {
        return invalid_channel();
    }
    match channel {
        // [CBE-Handbook p:460 s:17.10.4] MFC_RdTagStat reports the status from the last tag-group status update request.
        // [CBEA p:127 s:9.3.5] a read with no update request is a software-induced deadlock.
        // The model does not park the SPU on that deadlock and refuses the
        // read by name.
        spu::MFC_RD_TAG_STAT => match state.channels.tag_status_read.take() {
            Some(status) => {
                state.set_reg_channel_word(rt, status);
                SpuStepOutcome::Continue
            }
            None if state.channels.tag_update.is_some() => stall(),
            None => SpuStepOutcome::Fault(SpuFault::ChannelStall(channel)),
        },
        // [CBEA p:129 s:9.3.7] MFC_RdListStallStat returns the tag groups whose list stalled since the last read, clears them, and blocks while there are none.
        // A read never completes when the status is 0 and no list has a
        // stall still to come. The model refuses that read by name.
        spu::MFC_RD_LIST_STALL_STAT => {
            let status = core::mem::take(&mut state.channels.list_stall_status);
            if status != 0 {
                state.set_reg_channel_word(rt, status);
                SpuStepOutcome::Continue
            } else if state.channels.lists.iter().any(|list| !list.stalled) {
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
        // [CBE-Handbook p:462 s:17. SPE Channel and Related MMIO Interface sub:17.11 MFC Read Atomic Command Status Channel] Reports success/failure status for the most recent atomic command (e.g. putllc).
        spu::MFC_RD_ATOMIC_STAT => {
            state.set_reg_channel_word(rt, state.channels.atomic_status);
            state.channels.atomic_status_ready = false;
            SpuStepOutcome::Continue
        }
        // [CBEA p:141 s:9.8 SPU Read Machine Status Channel] Two status bits: IS (bit 30) isolation and IE (bit 31) interrupt enable; the model runs nonisolated with interrupts never enabled, so both read as zero.
        // The isolation facility is out of scope; docs/architecture/execution_units.md records why.
        spu::SPU_RD_MACH_STAT => {
            state.set_reg_channel_word(rt, 0);
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
        // [CBEA p:135 s:9.5.3] SPU_RdInMbox counts the messages in the inbound mailbox.
        spu::SPU_RD_IN_MBOX => u32::try_from(channels.in_mbox.len())
            .unwrap_or(u32::MAX)
            .min(spu::SPU_IN_MBOX_DEPTH),
        // [CBEA p:147 s:9.11.1] SPU_RdEventStat counts 1 once an enabled event is pending.
        // The model raises no SPU event.
        spu::SPU_RD_EVENT_STAT => 0,
        // [CBEA p:137 s:9.6.1], [CBEA p:138 s:9.6.2] a signal-notification channel counts 1 while unread signals are pending.
        spu::SPU_RD_SIG_NOTIFY_1 => u32::from(state.signals[0].pending),
        spu::SPU_RD_SIG_NOTIFY_2 => u32::from(state.signals[1].pending),
        // [CBEA p:129 s:9.3.7] MFC_RdListStallStat counts 1 once a list element with the stall-and-notify flag completes.
        spu::MFC_RD_LIST_STALL_STAT => u32::from(channels.list_stall_status != 0),
        // [CBEA p:130 s:9.3.8] MFC_WrListStallAck is nonblocking and counts 1.
        spu::MFC_WR_LIST_STALL_ACK => 1,
        // [CBEA p:134 s:9.5.2] SPU_WrOutIntrMbox counts its free entries.
        // The model writes nothing to it, so the one entry is free.
        spu::SPU_WR_OUT_INTR_MBOX => spu::SPU_OUT_INTR_MBOX_DEPTH,
        // [CBE-Handbook p:443 s:17.1.3] rchcnt on a reserved channel returns 0.
        _ if spu::is_reserved_channel(channel) => 0,
        _ => return None,
    })
}

/// Checks a transfer's latched parameters against `class`, then queues it with `ordering`.
fn issue_transfer(
    cmd: u32,
    direction: DmaDirection,
    class: MfcCommandClass,
    ordering: MfcOrdering,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    let tag = match checked_transfer(cmd, class, state, unit_id) {
        Ok(tag) => tag,
        Err(queued) => return queued,
    };
    let c = &state.channels;
    let ea = (u64::from(c.mfc_eah) << 32) | u64::from(c.mfc_eal);
    let (lsa, size) = (c.mfc_lsa, c.mfc_size);
    // A range past 2^64 names no segment: the queue raises it like any
    // address past the effective-address space.
    let Some(main) = ByteRange::new(GuestAddr::new(ea), u64::from(size)) else {
        return queue_invalid(cmd, MfcCommandError::DataSegment { ea }, state, unit_id);
    };
    let local =
        ByteRange::new(GuestAddr::new(u64::from(lsa)), u64::from(size)).expect("valid LS range");
    let (src, dst, payload) = match direction {
        // Each local-store byte's address wraps by the limit register, so
        // no range the guest stages escapes local store.
        DmaDirection::Put => (local, main, Some(state.read_ls_wrapped(lsa, size))),
        // The runtime reads a get's source when it completes and lands the
        // bytes in local store.
        DmaDirection::Get => (main, local, None),
    };
    // [CBEA p:65 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] Self-store overlapping the reserved line clears the reservation.
    if direction == DmaDirection::Put {
        if let Some(line) = state.reservation {
            if line.overlaps_range(ea, u64::from(size)) {
                state.reservation = None;
            }
        }
    }
    let request = DmaRequest::new(direction, src, dst, unit_id)
        .expect("matching sizes")
        .with_tag_id(tag)
        .with_ordering(ordering);
    state.channels.cmd_queue_free -= 1;
    SpuStepOutcome::Yield {
        effects: vec![Effect::DmaEnqueue { request, payload }],
        reason: YieldReason::DmaSubmitted,
    }
}

/// Queues an ordering command, a command that moves no bytes.
///
/// The command holds a queue slot and its tag group until the queue
/// completes it. The queue completes it after the commands its ordering
/// names:
///
/// - mfcsync and mfceieio order their tag group.
/// - The barrier command orders the whole queue.
///
/// [CBEA p:72 s:7.9.3] the barrier command is not tag-specific, and its tag says when it is complete.
/// [CBEA p:308 s:Appendix D Table D-4] the barrier does not order the immediate atomic commands.
fn issue_ordering_command(
    cmd: u32,
    ordering: MfcOrdering,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    let raw = state.channels.mfc_tag_id;
    let Some(tag) = u8::try_from(raw).ok().and_then(MfcTagId::new) else {
        // [CBE-Handbook p:456 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] A set bit above the tag field suspends MFC command queue processing
        return queue_invalid(cmd, MfcCommandError::ReservedTagBits(raw), state, unit_id);
    };
    let none = ByteRange::new(GuestAddr::new(0), 0).expect("an empty range");
    let request = DmaRequest::new(DmaDirection::Put, none, none, unit_id)
        .expect("matching sizes")
        .with_tag_id(tag)
        .with_ordering(ordering);
    state.channels.cmd_queue_free -= 1;
    SpuStepOutcome::Yield {
        effects: vec![Effect::DmaEnqueue {
            request,
            payload: Some(Vec::new()),
        }],
        reason: YieldReason::DmaSubmitted,
    }
}

/// Validate a list command's latched parameters, then queue its elements
/// up to the first stall-and-notify element.
///
/// The list takes one command-queue slot. A list that stops at a stall
/// keeps its slot and its tag group in [`ListCursor`] until it queues
/// its last element.
///
/// [CBEA p:60 s:7.5.3] the list address is in MFC_EAL and the list size in MFC_Size; each element names the transfer size and low effective address, and every element uses MFC_EAH.
/// [CBEA p:116 s:9.1.4] a list size may be 0; such a list names no transfer.
fn issue_list(
    cmd: u32,
    direction: DmaDirection,
    ordering: MfcOrdering,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    let tag = match checked_transfer(cmd, MfcCommandClass::List, state, unit_id) {
        Ok(tag) => tag,
        Err(queued) => return queued,
    };
    // [CBEA p:60 s:7.5.3] the LSA must be 16-byte aligned when the first element is 16 bytes or less.
    let c = &state.channels;
    if c.mfc_size > 0 {
        let head = state.read_ls_wrapped(c.mfc_eal, 4);
        let first =
            u32::from_be_bytes([head[0], head[1], head[2], head[3]]) & !spu::MFC_LIST_STALL_NOTIFY;
        if first <= 16 && c.mfc_lsa & 0xF != 0 {
            let error = MfcCommandError::LocalStoreUnaligned {
                lsa: c.mfc_lsa,
                size: first,
            };
            return queue_invalid(cmd, error, state, unit_id);
        }
    }
    let c = &state.channels;
    let mut list = ListCursor {
        word: cmd,
        tag,
        direction,
        ordering,
        eah: c.mfc_eah,
        element: c.mfc_eal,
        remaining: c.mfc_size / spu::MFC_LIST_ELEMENT_BYTES,
        data: c.mfc_lsa,
        stalled: false,
    };
    state.channels.cmd_queue_free -= 1;
    let effects = if list.remaining == 0 {
        vec![ordered_empty_command(tag, ordering, unit_id)]
    } else {
        queue_list_segment(cmd, &mut list, state, unit_id)
    };
    if list.remaining > 0 {
        state.channels.lists.push(list);
    }
    SpuStepOutcome::Yield {
        effects,
        reason: YieldReason::DmaSubmitted,
    }
}

/// A queued command of no length under `tag`: it holds a slot and its
/// tag group until the queue completes it.
fn ordered_empty_command(tag: MfcTagId, ordering: MfcOrdering, unit_id: UnitId) -> Effect {
    let none = ByteRange::new(GuestAddr::new(0), 0).expect("an empty range");
    let request = DmaRequest::new(DmaDirection::Put, none, none, unit_id)
        .expect("matching sizes")
        .with_tag_id(tag)
        .with_ordering(ordering);
    Effect::DmaEnqueue {
        request,
        payload: Some(Vec::new()),
    }
}

/// Queue the list's elements from its cursor up to and including the
/// next stall-and-notify element, or to its end.
///
/// The model reads each element from local store when it queues the
/// segment. An element that software rewrote while the list stalled is
/// the one the segment queues. The segment's last element holds the
/// list's slot when it ends the list; otherwise the list's
/// [`ListCursor`] holds it. An element the MFC refuses ends the list as
/// an invalid command, which takes over the slot.
///
/// [CBEA p:59 s:7.4] the elements are processed in order; each transfer starts at the next quadword boundary of local store after the last, and one below 16 bytes takes the low four bits of its effective address; a stall-and-notify flag on the last element is ignored.
/// [CBEA p:129 s:9.3.7] the MFC reads no element past one with the stall-and-notify flag until the stall is acknowledged.
fn queue_list_segment(
    cmd: u32,
    list: &mut ListCursor,
    state: &mut SpuState,
    unit_id: UnitId,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    while list.remaining > 0 {
        let raw = state.read_ls_wrapped(list.element, spu::MFC_LIST_ELEMENT_BYTES);
        let word = |i: usize| u32::from_be_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
        let (head, eal) = (word(0), word(4));
        list.element = list.element.wrapping_add(spu::MFC_LIST_ELEMENT_BYTES);
        list.remaining -= 1;
        let stall = head & spu::MFC_LIST_STALL_NOTIFY != 0 && list.remaining > 0;
        let size = head & !spu::MFC_LIST_STALL_NOTIFY;
        let lsa = if size < 16 {
            (list.data & !0xF) | (eal & 0xF)
        } else {
            list.data
        };
        let params = MfcParameters {
            lsa,
            eah: list.eah,
            eal,
            size,
            tag: u32::from(list.tag.raw()),
        };
        // [CBE-Handbook p:531 s:19.4.4.2] a list element transfer cannot cross the 4 GB area the list's EAH names; the DMA halts at the boundary and an MFC exception is signalled.
        // The model moves none of the element's bytes, as for an address
        // that does not translate.
        let crosses = u64::from(eal) + u64::from(size) > 1 << 32;
        let checked = cellgov_dma::validate(MfcCommandClass::Transfer, params)
            .and_then(|()| {
                if crosses {
                    Err(MfcCommandError::ListElementCrosses4Gb {
                        ea: params.ea(),
                        size,
                    })
                } else {
                    Ok(())
                }
            })
            .and_then(|()| {
                ByteRange::new(GuestAddr::new(params.ea()), u64::from(size))
                    .ok_or(MfcCommandError::DataSegment { ea: params.ea() })
            });
        let main = match checked {
            Ok(main) => main,
            // The documents name no exception for a bad element. The model
            // raises the one a single transfer with the same parameters
            // raises.
            Err(error) => {
                state.channels.cmd_queue_free += 1;
                effects.push(invalid_command(cmd, params, error, state, unit_id));
                list.remaining = 0;
                break;
            }
        };
        // A zero-size element skips a transfer, and leaves the next one
        // where it would have been.
        // [CBEA p:129 s:9.3.7] software skips a list element by setting its transfer size to zero.
        if size > 0 {
            list.data = lsa.wrapping_add(size).wrapping_add(15) & !15;
        }
        let local = ByteRange::new(GuestAddr::new(u64::from(lsa)), u64::from(size))
            .expect("valid LS range");
        let (src, dst, payload) = match list.direction {
            DmaDirection::Put => (local, main, Some(state.read_ls_wrapped(lsa, size))),
            DmaDirection::Get => (main, local, None),
        };
        // [CBEA p:65 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] Self-store overlapping the reserved line clears the reservation.
        if list.direction == DmaDirection::Put {
            if let Some(line) = state.reservation {
                if line.overlaps_range(params.ea(), u64::from(size)) {
                    state.reservation = None;
                }
            }
        }
        let mut request = DmaRequest::new(list.direction, src, dst, unit_id)
            .expect("matching sizes")
            .with_tag_id(list.tag)
            .with_ordering(list.ordering);
        if stall {
            request = request.with_stall_notify().without_slot();
        } else if list.remaining > 0 {
            request = request.without_slot();
        }
        effects.push(Effect::DmaEnqueue { request, payload });
        if stall {
            break;
        }
    }
    effects
}

/// Restart the stalled lists of the tag group `value` names.
///
/// For a group with no stalled list, the model restarts nothing.
///
/// [CBEA p:130 s:9.3.8] an acknowledgment for a tag group with no stalled list has undefined results.
fn acknowledge_list_stall(value: u32, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    let group = value & 0x1F;
    let mut effects = Vec::new();
    let mut lists = core::mem::take(&mut state.channels.lists);
    lists.retain_mut(|list| {
        if !list.stalled || u32::from(list.tag.raw()) != group {
            return true;
        }
        list.stalled = false;
        effects.extend(queue_list_segment(list.word, list, state, unit_id));
        list.remaining > 0
    });
    state.channels.lists = lists;
    if effects.is_empty() {
        return SpuStepOutcome::Continue;
    }
    SpuStepOutcome::Yield {
        effects,
        reason: YieldReason::DmaSubmitted,
    }
}

/// Checks a transfer's latched parameters against `class` and returns its tag group.
///
/// A command the MFC refuses still retires. It joins the queue as an
/// invalid command and takes a slot. The queue suspends when it reaches
/// that command. The `Err` is that step's outcome.
///
/// [CBEA p:113 s:9.1.1] the parameters' validity is checked asynchronous to the instruction stream.
fn checked_transfer(
    cmd: u32,
    class: MfcCommandClass,
    state: &mut SpuState,
    unit_id: UnitId,
) -> Result<MfcTagId, SpuStepOutcome> {
    let params = latched_parameters(state);
    let error = match cellgov_dma::validate(class, params) {
        Ok(()) => match u8::try_from(params.tag).ok().and_then(MfcTagId::new) {
            Some(tag) => return Ok(tag),
            // [CBE-Handbook p:456 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] A set bit above the tag field suspends MFC command queue processing
            None => MfcCommandError::ReservedTagBits(params.tag),
        },
        Err(error) => error,
    };
    Err(queue_invalid(cmd, error, state, unit_id))
}

/// The parameters the channels latched for the next command.
fn latched_parameters(state: &SpuState) -> MfcParameters {
    let c = &state.channels;
    MfcParameters {
        lsa: c.mfc_lsa,
        eah: c.mfc_eah,
        eal: c.mfc_eal,
        size: c.mfc_size,
        tag: c.mfc_tag_id,
    }
}

/// The effect that queues `cmd` as a command the MFC refuses with
/// `error`. It takes a slot like any queued command.
pub(crate) fn invalid_command(
    cmd: u32,
    params: MfcParameters,
    error: MfcCommandError,
    state: &mut SpuState,
    unit_id: UnitId,
) -> Effect {
    state.channels.cmd_queue_free -= 1;
    Effect::MfcInvalidCommand {
        issuer: unit_id,
        command: InvalidMfcCommand {
            word: cmd,
            params,
            error,
        },
    }
}

/// Queue `cmd` as a command the MFC refuses with `error`.
fn queue_invalid(
    cmd: u32,
    error: MfcCommandError,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    SpuStepOutcome::Yield {
        effects: vec![invalid_command(
            cmd,
            latched_parameters(state),
            error,
            state,
            unit_id,
        )],
        reason: YieldReason::DmaSubmitted,
    }
}

/// The data-segment exception an atomic command raises for an effective
/// address past the Cell's real-address bound, if it does.
///
/// CellGov models no segment table. It treats such an address as one no
/// segment names, as the runtime's translation check does for a
/// transfer. Naming its line would also break the reservation line's
/// own bound.
/// [CBEA p:120 s:9.1.7] a segment fault suspends the queue and raises the MFC data-segment interrupt.
fn atomic_segment_fault(ea: u64) -> Option<MfcCommandError> {
    (ea > CELL_EA_LIMIT).then_some(MfcCommandError::DataSegment { ea })
}

/// The command error an opcode alone raises on the SPU queue, if any.
///
/// [CBEA p:57 s:7.2 Table 7-6] an invalid opcode, a reserved bit in the opcode, and an `s` command on the SPU queue are DMA command errors.
/// [CBE-Handbook p:287 s:9.8.3.2] the CBE checks the reserved bits: nonzero upper 8 opcode bits are a DMA command error.
fn opcode_error(word: MfcCmd) -> Option<MfcCommandError> {
    match word.class() {
        MfcOpcodeClass::Reserved => Some(MfcCommandError::ReservedOpcode(word.opcode())),
        MfcOpcodeClass::Illegal => Some(MfcCommandError::IllegalOpcode(word.opcode())),
        MfcOpcodeClass::Defined(def) if def.queues == MfcQueues::ProxyOnly => {
            Some(MfcCommandError::ProxyOnlyCommand(word.opcode()))
        }
        MfcOpcodeClass::Defined(_) => None,
    }
}

/// The barrier a queued MFC command word is, if its opcode orders the
/// queue.
pub(crate) fn mfc_barrier_kind(cmd: u32) -> Option<BarrierKind> {
    Some(match MfcCmd::new(cmd).opcode() {
        spu::MFC_PUTF | spu::MFC_PUTRF | spu::MFC_GETF | spu::MFC_SNDSIGF => BarrierKind::MfcFence,
        spu::MFC_PUTLF | spu::MFC_PUTRLF | spu::MFC_GETLF => BarrierKind::MfcFence,
        spu::MFC_PUTB | spu::MFC_PUTRB | spu::MFC_GETB | spu::MFC_SNDSIGB => {
            BarrierKind::MfcTagBarrier
        }
        spu::MFC_PUTLB | spu::MFC_PUTRLB | spu::MFC_GETLB => BarrierKind::MfcTagBarrier,
        spu::MFC_SYNC => BarrierKind::MfcSync,
        spu::MFC_EIEIO => BarrierKind::MfcEieio,
        spu::MFC_BARRIER => BarrierKind::MfcBarrier,
        _ => return None,
    })
}

fn execute_mfc_cmd(cmd: u32, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    // [CBEA p:113 s:9.1.1] a write to MFC_Cmd with the command queue full stalls until a slot frees.
    // [CBEA p:65 s:7.8] the immediate atomic commands also need a free slot, though they are not queued behind other commands.
    if state.channels.cmd_queue_free == 0 {
        return stall();
    }
    let word = MfcCmd::new(cmd);
    // [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] an invalid command suspends queue processing and raises an invalid-command interrupt, and the leading bit of the command halfword marks the opcode reserved.
    // A command error outranks the parameter checks, so this check runs
    // first.
    if let Some(error) = opcode_error(word) {
        return queue_invalid(cmd, error, state, unit_id);
    }
    // The two class ids ride in the same word and steer bus bandwidth
    // and cache replacement. They change how fast a command runs, never
    // what it does. This model has neither to steer, so it reads past
    // them.
    // [CBEA p:114 s:9.1.2 MFC Class ID Channel] a class id is never checked, an unrecognised one falls back to the default, and none of them raises an exception.
    let ea = ((state.channels.mfc_eah as u64) << 32) | state.channels.mfc_eal as u64;
    let lsa = state.channels.mfc_lsa;
    // [CBEA p:66 s:7.8.1] the getllar data transfer is one cache line.
    // Which local-store line an unaligned MFC_LSA names is unestablished.
    // This model takes the line that contains it, as it does for the
    // effective address. A line therefore never straddles the end of
    // local store.
    let line_lsa = lsa & !(RESERVATION_LINE_BYTES as u32 - 1);

    match word.opcode() {
        // [CBEA p:61 s:7. MFC Commands sub:7.6 Put Commands (Local Storage to Main Storage)] put: copy LS bytes to main storage.
        // [CBEA p:60 s:7. MFC Commands sub:7.5 Get Commands (Main Storage to Local Storage)] get: copy main-storage bytes into LS.
        // The fence and barrier forms move the same bytes and differ only
        // in when the queue may complete them.
        // [CBEA p:62 s:7.6.5] the CBE does not implement the result hint and runs putr, putrf and putrb as put, putf and putb.
        spu::MFC_PUT | spu::MFC_PUTR => {
            issue_transfer(cmd, Put, Transfer, Order::None, state, unit_id)
        }
        spu::MFC_PUTF | spu::MFC_PUTRF => {
            issue_transfer(cmd, Put, Transfer, Order::Fence, state, unit_id)
        }
        spu::MFC_PUTB | spu::MFC_PUTRB => {
            issue_transfer(cmd, Put, Transfer, Order::TagBarrier, state, unit_id)
        }
        spu::MFC_GET => issue_transfer(cmd, Get, Transfer, Order::None, state, unit_id),
        spu::MFC_GETF => issue_transfer(cmd, Get, Transfer, Order::Fence, state, unit_id),
        spu::MFC_GETB => issue_transfer(cmd, Get, Transfer, Order::TagBarrier, state, unit_id),
        // [CBEA p:308 s:Appendix D Table D-4] sndsig is a 4-byte DMA put that can go to any address.
        // Delivery to another SPU's signal-notification register needs
        // that SPU's problem-state alias. This model does not map that
        // alias, so the put lands at its effective address in main storage.
        spu::MFC_SNDSIG => issue_transfer(cmd, Put, SendSignal, Order::None, state, unit_id),
        spu::MFC_SNDSIGF => issue_transfer(cmd, Put, SendSignal, Order::Fence, state, unit_id),
        spu::MFC_SNDSIGB => issue_transfer(cmd, Put, SendSignal, Order::TagBarrier, state, unit_id),
        // [CBEA p:71 s:7.9.1] mfcsync creates a tag-specific barrier even though it has no b modifier.
        // [CBEA p:72 s:7.9.2] so does mfceieio.
        spu::MFC_SYNC | spu::MFC_EIEIO => {
            issue_ordering_command(cmd, Order::TagBarrier, state, unit_id)
        }
        spu::MFC_BARRIER => issue_ordering_command(cmd, Order::QueueBarrier, state, unit_id),
        // [CBEA p:58 s:7.4] a list command runs a sequence of transfers, one per list element, under one tag group.
        // [CBEA p:62 s:7.6.5] on the CBE the putrl forms behave as the putl forms.
        spu::MFC_PUTL | spu::MFC_PUTRL => issue_list(cmd, Put, Order::None, state, unit_id),
        spu::MFC_PUTLF | spu::MFC_PUTRLF => issue_list(cmd, Put, Order::Fence, state, unit_id),
        spu::MFC_PUTLB | spu::MFC_PUTRLB => issue_list(cmd, Put, Order::TagBarrier, state, unit_id),
        spu::MFC_GETL => issue_list(cmd, Get, Order::None, state, unit_id),
        spu::MFC_GETLF => issue_list(cmd, Get, Order::Fence, state, unit_id),
        spu::MFC_GETLB => issue_list(cmd, Get, Order::TagBarrier, state, unit_id),
        // [CBEA p:66 s:7.8.1 Get Lock Line and Reserve Command] getllar: the transfer is one cache line, placed in local storage, with a reservation over it.
        // The effective address names the line by any byte inside it.
        // [CBEA p:57 s:7.2 Command Exceptions] alignment is not checked for the atomic commands, so a misaligned address refuses nothing.
        // The bytes and the reservation therefore both cover the
        // containing line. The caller writes the reservation register
        // and the status channel after the line arrives; see
        // `SpuStepOutcome::MemoryRead`.
        // [CBEA p:57 s:7.2 Table 7-6] a getllar, putllc or putlluc issued while another is pending is a command error.
        // No atomic command is ever pending here. The unit copies a
        // getllar's line in the step that issues it. A putllc yields its
        // store to the step's commit before the next instruction runs.
        spu::MFC_GETLLAR => {
            if let Some(error) = atomic_segment_fault(ea) {
                return queue_invalid(cmd, error, state, unit_id);
            }
            let line = cellgov_sync::ReservedLine::containing(ea);
            SpuStepOutcome::MemoryRead {
                ea: line.addr(),
                lsa: line_lsa,
                size: RESERVATION_LINE_BYTES as u32,
                acquire_line: Some(line.addr()),
            }
        }
        // [CBEA p:66 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] putllc: conditional store that succeeds only if the local reservation is still held for this line.
        spu::MFC_PUTLLC => {
            if let Some(error) = atomic_segment_fault(ea) {
                return queue_invalid(cmd, error, state, unit_id);
            }
            let line = cellgov_sync::ReservedLine::containing(ea);
            let success = match state.reservation {
                Some(l) => l.addr() == line.addr(),
                None => false,
            };
            if success {
                let ls_bytes = state.read_ls_wrapped(line_lsa, RESERVATION_LINE_BYTES as u32);
                state.reservation = None;
                // The store covers the line the reservation named, as
                // the getllar arm's read did.
                let range = ByteRange::new(GuestAddr::new(line.addr()), RESERVATION_LINE_BYTES)
                    .expect("valid EA range");
                state.channels.atomic_status = 0;
                state.channels.atomic_status_ready = true;
                SpuStepOutcome::Yield {
                    effects: vec![Effect::ConditionalStore {
                        range,
                        bytes: WritePayload::new(ls_bytes),
                        source: unit_id,
                        source_time: GuestTicks::ZERO,
                    }],
                    reason: YieldReason::DmaSubmitted,
                }
            } else {
                state.reservation = None;
                state.channels.atomic_status = MFC_ATOMIC_STAT_S;
                state.channels.atomic_status_ready = true;
                SpuStepOutcome::Continue
            }
        }
        // A defined command the SPU queue accepts and this model does not
        // run. The fault marks a gap in the model.
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedMfcCommand(cmd)),
    }
}

#[cfg(test)]
#[path = "tests/mfc_class_id_tests.rs"]
mod mfc_class_id_tests;

#[cfg(test)]
#[path = "tests/mfc_opcode_class_tests.rs"]
mod mfc_opcode_class_tests;

#[cfg(test)]
#[path = "tests/mfc_parameter_latch_tests.rs"]
mod mfc_parameter_latch_tests;

#[cfg(test)]
#[path = "tests/mfc_ordering_form_tests.rs"]
mod mfc_ordering_form_tests;

#[cfg(test)]
#[path = "tests/mfc_list_tests.rs"]
mod mfc_list_tests;
