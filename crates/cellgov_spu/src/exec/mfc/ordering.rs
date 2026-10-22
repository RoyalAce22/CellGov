//! MFC ordering and storage-control commands: the barriers, putqlluc,
//! and the SL1 commands.

use crate::state::SpuState;
use cellgov_dma::{DmaDirection, DmaRequest, MfcCommandClass, MfcCommandError, MfcOrdering};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::MfcTagId;
use cellgov_sync::RESERVATION_LINE_BYTES;

use super::command::queue_invalid;
use super::transfer::{atomic_segment_fault, checked_transfer, local_store_transfer};
use crate::exec::outcome::SpuStepOutcome;

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
pub(super) fn issue_ordering_command(
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
        .with_ordering(ordering)
        .with_command_word(cmd);
    state.channels.cmd_queue_free -= 1;
    SpuStepOutcome::Yield {
        effects: vec![Effect::DmaEnqueue {
            request,
            payload: Some(Vec::new()),
        }],
        reason: YieldReason::DmaSubmitted,
    }
}

/// Queues a putqlluc: a put of the local-store line at `line_lsa` over
/// the line that contains `ea`, behind a tag-specific fence.
///
/// It reads its line when it completes, as every queued put does, and it
/// reports no atomic status: its tag group says when it is complete.
///
/// [CBEA p:68 s:7.8.4] putqlluc is putlluc placed in the command queue; it creates a tag-specific fence and completes through its tag group.
/// [CBEA p:131 s:9.4] MFC_RdAtomicStat gives no status for the queued putqlluc.
pub(super) fn issue_queued_lock_line(
    cmd: u32,
    ea: u64,
    line_lsa: u32,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    let raw = state.channels.mfc_tag_id;
    let Some(tag) = u8::try_from(raw).ok().and_then(MfcTagId::new) else {
        // [CBE-Handbook p:456 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] A set bit above the tag field suspends MFC command queue processing
        return queue_invalid(cmd, MfcCommandError::ReservedTagBits(raw), state, unit_id);
    };
    if let Some(error) = atomic_segment_fault(ea) {
        return queue_invalid(cmd, error, state, unit_id);
    }
    let line = cellgov_sync::ReservedLine::containing(ea);
    let main = ByteRange::new(GuestAddr::new(line.addr()), RESERVATION_LINE_BYTES)
        .expect("valid EA range");
    let local = ByteRange::new(GuestAddr::new(u64::from(line_lsa)), RESERVATION_LINE_BYTES)
        .expect("valid LS range");
    // [CBE-Handbook p:479 s:18.6.4] a matching putllc, putlluc or putqlluc resets the SPE's reservation, and raises no lock-line reservation lost event.
    if state
        .reservation()
        .is_some_and(|held| held.addr() == line.addr())
    {
        state.set_reservation(None);
    }
    let request = local_store_transfer(DmaDirection::Put, local, main, unit_id)
        .with_tag_id(tag)
        .with_ordering(MfcOrdering::Fence)
        .with_command_word(cmd);
    state.channels.cmd_queue_free -= 1;
    SpuStepOutcome::Yield {
        effects: vec![Effect::DmaEnqueue {
            request,
            payload: None,
        }],
        reason: YieldReason::DmaSubmitted,
    }
}

/// Queues an SL1 storage control command as a tag-specific barrier.
///
/// With `zero`, the command writes zeros over every data block its
/// range touches. Otherwise it moves no bytes: CellGov's main storage has
/// no cache for a store or flush to write back.
///
/// [CBEA p:62 s:7.7] the storage control commands affect the bytes the transfer size names, and each creates a tag-specific barrier.
/// [CBEA p:64 s:7.7.3] sdcrz sets to zero every byte of the data block that contains an addressed byte.
pub(super) fn issue_storage_control(
    cmd: u32,
    zero: bool,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
    let tag = match checked_transfer(cmd, MfcCommandClass::StorageControl, state, unit_id) {
        Ok(tag) => tag,
        Err(queued) => return queued,
    };
    let c = &state.channels;
    let ea = (u64::from(c.mfc_eah) << 32) | u64::from(c.mfc_eal);
    let size = if zero { u64::from(c.mfc_size) } else { 0 };
    let start = ea & !(spu::MFC_SL1_DATA_BLOCK_BYTES - 1);
    let end = if size == 0 {
        Some(start)
    } else {
        ea.checked_add(size)
            .and_then(|end| end.checked_next_multiple_of(spu::MFC_SL1_DATA_BLOCK_BYTES))
    };
    // A block past 2^64 names no segment.
    let Some(blocks) = end.and_then(|end| ByteRange::new(GuestAddr::new(start), end - start))
    else {
        return queue_invalid(cmd, MfcCommandError::DataSegment { ea }, state, unit_id);
    };
    // The zeros ride as the put's payload, so no local store is read. The
    // source names local-store address 0, the address a record of the
    // command gives when it names none.
    let none = ByteRange::new(GuestAddr::new(0), blocks.length()).expect("valid LS range");
    let request = DmaRequest::new(DmaDirection::Put, none, blocks, unit_id)
        .expect("matching sizes")
        .with_tag_id(tag)
        .with_ordering(MfcOrdering::TagBarrier)
        .with_command_word(cmd);
    state.channels.cmd_queue_free -= 1;
    SpuStepOutcome::Yield {
        effects: vec![Effect::DmaEnqueue {
            request,
            payload: Some(vec![0; blocks.length() as usize]),
        }],
        reason: YieldReason::DmaSubmitted,
    }
}

/// A queued command `cmd` of no length under `tag`: it holds a slot and
/// its tag group until the queue completes it.
pub(super) fn ordered_empty_command(
    cmd: u32,
    tag: MfcTagId,
    ordering: MfcOrdering,
    unit_id: UnitId,
) -> Effect {
    let none = ByteRange::new(GuestAddr::new(0), 0).expect("an empty range");
    let request = DmaRequest::new(DmaDirection::Put, none, none, unit_id)
        .expect("matching sizes")
        .with_tag_id(tag)
        .with_ordering(ordering)
        .with_command_word(cmd);
    Effect::DmaEnqueue {
        request,
        payload: Some(Vec::new()),
    }
}

#[cfg(test)]
#[path = "tests/mfc_ordering_form_tests.rs"]
mod mfc_ordering_form_tests;

#[cfg(test)]
#[path = "tests/mfc_storage_control_tests.rs"]
mod mfc_storage_control_tests;
