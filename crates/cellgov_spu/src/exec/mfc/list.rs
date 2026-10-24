//! MFC list commands: list start, the held lists, element queueing, and
//! the stall acknowledgement.

use crate::state::{ListCursor, SpuState};
use cellgov_dma::{
    DmaDirection, DmaRequest, MfcCommandClass, MfcCommandError, MfcOrdering, MfcParameters,
};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{BarrierKind, MfcOrderingView, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::MfcCmd;

use super::command::{invalid_command, mfc_barrier_kind};
use super::ordering::ordered_empty_command;
use super::transfer::{checked_transfer, local_store_transfer};
use crate::exec::outcome::SpuStepOutcome;

/// Validate a list command's latched parameters, then start the list or hold it.
///
/// [`list_waits`] decides whether the list waits behind the commands its
/// ordering names. The list takes one command-queue slot. [`ListCursor`]
/// keeps the slot and the tag group until the list queues its last
/// element, while the list is one of:
///
/// - stopped at a stall-and-notify element;
/// - held behind another command.
///
/// [CBEA p:60 s:7.5.3] the list address is in MFC_EAL and the list size in MFC_Size; each element names the transfer size and low effective address, and every element uses MFC_EAH.
/// [CBEA p:116 s:9.1.4] a list size may be 0; such a list names no transfer.
pub(super) fn issue_list(
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
        held: false,
    };
    // The list reads its elements when it starts. An earlier get in its
    // tag group can still be in flight with those elements.
    // In the `spu_list_ordered_read` console capture, a getlb issued
    // straight after a get of its own list moves what the new list names.
    // The capture leaves the getl's word volatile, because its read is
    // unordered against the get.
    let effects = if list_waits(&list, c.mfc_ordering, &c.lists) {
        list.held = true;
        state.channels.cmd_queue_free -= 1;
        state.channels.lists.push(list);
        Vec::new()
    } else {
        start_list(list, state, unit_id)
    };
    SpuStepOutcome::Yield {
        effects,
        reason: YieldReason::DmaSubmitted,
    }
}

/// Whether an earlier command orders `list` after itself, so `list` reads no element yet.
///
/// The earlier command is one in `queued`, or a list of the same tag
/// group in `ahead`. `list` waits for:
///
/// - every earlier command of its tag group, when `list` is a fence or
///   tag-barrier list;
/// - a tag barrier of its tag group;
/// - a barrier command.
///
/// [`cellgov_dma::DmaQueue::ordering_floor`] holds queued transfers to the
/// same orderings.
///
/// [CBEA p:69 s:7.9] a fence orders the command after every preceding command in its tag group.
/// [CBEA p:69 s:7.9] a barrier orders the command and every later command of its tag group after the preceding commands of the group.
/// [CBEA p:72 s:7.9.3] subsequent commands in the queue begin when the barrier command completes.
fn list_waits(list: &ListCursor, queued: MfcOrderingView, ahead: &[ListCursor]) -> bool {
    let bit = list.tag.status_bit();
    let mut group = ahead.iter().filter(|other| other.tag == list.tag);
    let ordered = matches!(list.ordering, MfcOrdering::Fence | MfcOrdering::TagBarrier);
    queued.queue_barrier
        || queued.barrier_tags & bit != 0
        || (ordered && queued.outstanding_tags & bit != 0)
        || if ordered {
            group.next().is_some()
        } else {
            group.any(|other| other.ordering == MfcOrdering::TagBarrier)
        }
}

/// Read `list`'s elements and queue them up to the first stall-and-notify element.
///
/// The list takes its command-queue slot here. A list that stops at a
/// stall joins the unit's lists.
fn start_list(mut list: ListCursor, state: &mut SpuState, unit_id: UnitId) -> Vec<Effect> {
    // [CBEA p:60 s:7.5.3] the LSA must be 16-byte aligned when the first element is 16 bytes or less.
    if list.remaining > 0 {
        let head = state.read_ls_wrapped(list.element, 4);
        let first =
            u32::from_be_bytes([head[0], head[1], head[2], head[3]]) & !spu::MFC_LIST_STALL_NOTIFY;
        if first <= 16 && list.data & 0xF != 0 {
            let params = MfcParameters {
                lsa: list.data,
                eah: list.eah,
                eal: list.element,
                size: list.remaining * spu::MFC_LIST_ELEMENT_BYTES,
                tag: u32::from(list.tag.raw()),
            };
            let error = MfcCommandError::LocalStoreUnaligned {
                lsa: list.data,
                size: first,
            };
            return vec![invalid_command(list.word, params, error, state, unit_id)];
        }
    }
    state.channels.cmd_queue_free = state.channels.cmd_queue_free.saturating_sub(1);
    let effects = if list.remaining == 0 {
        vec![ordered_empty_command(
            list.word,
            list.tag,
            list.ordering,
            unit_id,
        )]
    } else {
        queue_list_segment(list.word, &mut list, state, unit_id)
    };
    if list.remaining > 0 {
        state.channels.lists.push(list);
    }
    effects
}

/// Start the held lists that no earlier command still orders, oldest first.
///
/// The runtime calls this between the unit's steps. The step entry then
/// recounts the free command-queue slots, so this pass leaves the count
/// as it found it.
pub(crate) fn start_held_lists(
    state: &mut SpuState,
    unit_id: UnitId,
    mut queued: MfcOrderingView,
) -> Vec<Effect> {
    if !state.channels.lists.iter().any(|list| list.held) {
        return Vec::new();
    }
    let free = state.channels.cmd_queue_free;
    let mut effects = Vec::new();
    for mut list in core::mem::take(&mut state.channels.lists) {
        if !list.held || list_waits(&list, queued, &state.channels.lists) {
            state.channels.lists.push(list);
            continue;
        }
        list.held = false;
        // The queue holds a list this pass starts ahead of the lists after
        // it. `queued`, taken before the pass, does not show that list.
        let bit = list.tag.status_bit();
        queued.outstanding_tags |= bit;
        if list.ordering == MfcOrdering::TagBarrier {
            queued.barrier_tags |= bit;
        }
        effects.extend(start_list(list, state, unit_id));
    }
    state.channels.cmd_queue_free = free;
    effects
}

/// Whether a held list orders the command word `cmd` after itself.
///
/// The queue orders a command only against commands it holds. A command
/// that a held list orders waits on `MFC_Cmd` until the list starts:
///
/// - a fence or tag-barrier command waits for any held list of its tag
///   group;
/// - every command of the tag group waits for a held tag-barrier list;
/// - the barrier command waits for every held list.
///
/// The queue holds no immediate atomic command, so those wait for none.
///
/// [CBEA p:69 s:7.9] a barrier orders the command and every later command of its tag group after the preceding commands of the group.
pub(super) fn held_list_orders(cmd: u32, state: &SpuState) -> bool {
    let opcode = MfcCmd::new(cmd).opcode();
    if matches!(
        opcode,
        spu::MFC_GETLLAR | spu::MFC_PUTLLC | spu::MFC_PUTLLUC
    ) {
        return false;
    }
    let ordering = match mfc_barrier_kind(cmd) {
        Some(BarrierKind::MfcFence) => MfcOrdering::Fence,
        Some(BarrierKind::MfcTagBarrier | BarrierKind::MfcSync | BarrierKind::MfcEieio) => {
            MfcOrdering::TagBarrier
        }
        Some(BarrierKind::MfcBarrier) => MfcOrdering::QueueBarrier,
        Some(_) | None => MfcOrdering::None,
    };
    let tag = state.channels.mfc_tag_id;
    state
        .channels
        .lists
        .iter()
        .filter(|list| list.held)
        .any(|list| {
            ordering == MfcOrdering::QueueBarrier
                || (u32::from(list.tag.raw()) == tag
                    && (ordering != MfcOrdering::None || list.ordering == MfcOrdering::TagBarrier))
        })
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
        let (head, eal) = read_list_element(state, list);
        let stall = head & spu::MFC_LIST_STALL_NOTIFY != 0 && list.remaining > 0;
        let size = head & !spu::MFC_LIST_STALL_NOTIFY;
        let lsa = element_lsa(list.data, eal, size);
        let params = MfcParameters {
            lsa,
            eah: list.eah,
            eal,
            size,
            tag: u32::from(list.tag.raw()),
        };
        let main = match check_list_element(params) {
            Ok(main) => main,
            // The documents name no exception for a bad element. The
            // spu_list_bad_size and spu_list_bad_ea console captures show
            // the one a single transfer with the same parameters raises,
            // with the elements before it landed and none after it.
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
        let request = element_request(list, lsa, size, main, stall, unit_id);
        effects.push(Effect::DmaEnqueue {
            request,
            payload: None,
        });
        if stall {
            break;
        }
    }
    effects
}

/// Read the element at `list`'s cursor and move the cursor past it.
///
/// Returns the element's size word, with its stall-and-notify flag, and
/// its low effective address.
fn read_list_element(state: &SpuState, list: &mut ListCursor) -> (u32, u32) {
    let raw = state.read_ls_wrapped(list.element, spu::MFC_LIST_ELEMENT_BYTES);
    let word = |i: usize| u32::from_be_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
    list.element = list.element.wrapping_add(spu::MFC_LIST_ELEMENT_BYTES);
    list.remaining -= 1;
    (word(0), word(4))
}

/// The local-store address of an element of `size` bytes at `eal`, when
/// the list's next transfer starts at `data`.
fn element_lsa(data: u32, eal: u32, size: u32) -> u32 {
    if size < 16 {
        (data & !0xF) | (eal & 0xF)
    } else {
        data
    }
}

/// The main-storage range of one list element, or the error the MFC
/// raises for it.
fn check_list_element(params: MfcParameters) -> Result<ByteRange, MfcCommandError> {
    cellgov_dma::validate(MfcCommandClass::Transfer, params)?;
    // [CBE-Handbook p:531 s:19.4.4.2] a list element transfer cannot cross the 4 GB area the list's EAH names; the DMA halts at the boundary and an MFC exception is signalled.
    // The model moves none of the element's bytes, as for an address
    // that does not translate.
    if u64::from(params.eal) + u64::from(params.size) > 1 << 32 {
        return Err(MfcCommandError::ListElementCrosses4Gb {
            ea: params.ea(),
            size: params.size,
        });
    }
    ByteRange::new(GuestAddr::new(params.ea()), u64::from(params.size))
        .ok_or(MfcCommandError::DataSegment { ea: params.ea() })
}

/// The transfer request for one list element. A stall-and-notify element
/// and every element before the list's last leave the slot to the list.
fn element_request(
    list: &ListCursor,
    lsa: u32,
    size: u32,
    main: ByteRange,
    stall: bool,
    unit_id: UnitId,
) -> DmaRequest {
    let local =
        ByteRange::new(GuestAddr::new(u64::from(lsa)), u64::from(size)).expect("valid LS range");
    let request = local_store_transfer(list.direction, local, main, unit_id)
        .with_tag_id(list.tag)
        .with_ordering(list.ordering)
        .with_command_word(list.word);
    if stall {
        request.with_stall_notify().without_slot()
    } else if list.remaining > 0 {
        request.without_slot()
    } else {
        request
    }
}

/// Restart the stalled lists of the tag group `value` names.
///
/// For a group with no stalled list, the model restarts nothing.
///
/// [CBEA p:130 s:9.3.8] an acknowledgment for a tag group with no stalled list has undefined results.
pub(in crate::exec) fn acknowledge_list_stall(
    value: u32,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
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

#[cfg(test)]
#[path = "tests/mfc_list_tests.rs"]
mod mfc_list_tests;
