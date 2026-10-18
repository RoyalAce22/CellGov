//! The parts of one SPU step that do not depend on the pipeline:
//!
//! - the state refresh at step entry
//! - interrupt entry
//! - one instruction, with the event updates its channel access makes
//! - the completion of an atomic read
//!
//! The execution unit runs them in its loop. A caller that executes
//! instructions one at a time runs the same functions.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::ExecutionContext;
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::MFC_ATOMIC_STAT_G;

use super::transfer::{copy_into_local_store, shared_read};
use crate::exec::{self, SpuStepOutcome};
use crate::instruction::SpuInstruction;
use crate::state::SpuState;

/// Brings `state` up to date with the world `ctx` shows at step entry.
///
/// A second call with the same context changes nothing.
pub fn enter_step(state: &mut SpuState, unit: UnitId, ctx: &ExecutionContext<'_>) {
    // A group reads complete when this unit has no outstanding
    // transfer with its tag, so a reused tag reads incomplete until its
    // new transfer lands.
    // [CBEA p:128 s:9.3.6] a set bit means the group has no outstanding operations.
    // A list with elements still to queue holds its tag group too.
    let channels = &mut state.channels;
    let list_tags = channels
        .lists
        .iter()
        .fold(0, |tags, list| tags | list.tag.status_bit());
    channels.tag_status = !(ctx.outstanding_dma_tags() | list_tags);
    // Each queued command holds a slot until it completes, and a list
    // holds its one slot until it queues its last element. A count
    // that rises from 0 here is the Qv event's edge.
    let held = u32::try_from(channels.lists.len()).unwrap_or(u32::MAX);
    channels.cmd_queue_free = spu::MFC_SPU_QUEUE_DEPTH
        .saturating_sub(ctx.dma_queue_occupancy())
        .saturating_sub(held);
    // A list stalls once its stall-and-notify element leaves the queue.
    // [CBEA p:129 s:9.3.7] the stall occurs after the flagged element's transfer completes, and sets the group's bit in MFC_RdListStallStat.
    for list in channels.lists.iter_mut().filter(|list| !list.stalled) {
        let bit = list.tag.status_bit();
        if ctx.list_stall_tags() & bit == 0 {
            list.stalled = true;
            channels.list_stall_status |= bit;
        }
    }
    state.channels.settle_tag_update();
    state.channels.in_mbox = ctx.inbound_mailbox().to_vec();
    // A multisource synchronization request completes once the queue
    // holds none of the transfers it tracks. A request this step makes
    // tracks the transfers the queue holds now.
    // [CBEA p:143 s:9.10] the count returns to 1 when the tracked transfers complete.
    let oldest = ctx.oldest_mfc_transfer();
    let channels = &mut state.channels;
    if channels
        .mssync_tracking
        .is_some_and(|upto| oldest.is_none_or(|seq| seq >= upto))
    {
        channels.mssync_tracking = None;
    }
    channels.mssync_horizon = oldest.map(|_| ctx.dma_next_sequence());
    // Every source count the step entry refreshed can have risen.
    state.set_clock(ctx.current_tick());
    state.update_events();

    // Mirror cross-unit reservation invalidation. The context view is
    // frozen for the step, so a single entry-time check suffices.
    // A store from another unit, or the landing of this unit's own
    // MFC put, drops the committed entry while the local register
    // holds a line. putllc, putlluc and putqlluc clear the register
    // as they issue.
    // [CBEA p:148 s:9.11.1] Lr is set when a snoop external to the MFC resets the reservation, and never for a local action.
    if state.reservation().is_some() && !ctx.reservation_held(unit) {
        state.set_reservation(None);
        state.raise_events(spu::event::LR);
    }
}

/// Takes a pending interrupt before the next instruction; returns whether it took one.
///
/// An interrupt is the one way the unit sees the decrementer pass zero
/// between two channel accesses.
pub fn take_pending_interrupt(state: &mut SpuState) -> bool {
    if state.interrupts_enabled() {
        state.sample_decrementer();
    }
    // [SPU-ISA p:251 s:12.1] an interrupt lands between instructions.
    if !state.interrupt_pending() {
        return false;
    }
    state.take_interrupt();
    true
}

/// Executes `insn` against `state` as the execution unit's loop does.
///
/// A channel access samples the decrementer first. A channel read or
/// write then updates the event sources.
pub fn execute_instruction(
    insn: &SpuInstruction,
    state: &mut SpuState,
    unit: UnitId,
) -> SpuStepOutcome {
    let channel_access = matches!(
        insn,
        SpuInstruction::Rdch { .. } | SpuInstruction::Wrch { .. } | SpuInstruction::Rchcnt { .. }
    );
    // The decrementer runs between instructions, so a channel access
    // first looks for the zero crossing it can report.
    if channel_access {
        state.sample_decrementer();
    }
    let outcome = exec::execute(insn, state, unit);
    // A channel access is the one instruction that moves an event
    // source's count within a step.
    if matches!(
        insn,
        SpuInstruction::Rdch { .. } | SpuInstruction::Wrch { .. }
    ) {
        state.update_events();
    }
    outcome
}

/// How an atomic read completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryReadCompletion {
    /// No region backs the line: the command moved nothing and raised
    /// the data-segment exception.
    Refused,
    /// The line landed in local store.
    Read {
        /// Whether the read took a reservation.
        acquired: bool,
    },
}

/// Completes the read a [`SpuStepOutcome::MemoryRead`] names.
///
/// The read appends its effects to `effects`, and takes a reservation
/// when `acquire_line` names a line. The caller advances the program
/// counter after the read.
#[allow(clippy::too_many_arguments)]
pub fn complete_memory_read(
    state: &mut SpuState,
    memory: &cellgov_mem::GuestMemory,
    unit: UnitId,
    ea: u64,
    lsa: u32,
    size: u32,
    acquire_line: Option<u64>,
    effects: &mut Vec<Effect>,
) -> MemoryReadCompletion {
    // `ea` comes from MFC_EAH and MFC_EAL, so the guest can
    // name an address no region backs.
    // [CBEA p:111 s:9 SPU Channel Map] MFC_EAL is a write channel carrying the low-order SPU effective-address command parameter.
    if copy_into_local_store(state, memory, ea, lsa, size).is_err() {
        // No region backs the line: the MFC raises the
        // data-segment exception for the getllar, which
        // moves no line and never reports its status. The
        // spu_ea_bound console capture shows that class
        // for a getllar at an address nothing maps.
        // [CBEA p:120 s:9.1.7] a segment fault suspends the queue and raises the MFC data-segment interrupt.
        // The command acquires no reservation over a
        // line it never read. One an earlier getllar took
        // stands, in the register as in the committed
        // table that getllar's acquire reaches.
        // The record names the line the command moves.
        let params = cellgov_dma::MfcParameters {
            lsa,
            eah: (ea >> 32) as u32,
            eal: ea as u32,
            size,
            tag: state.channels.mfc_tag_id,
        };
        effects.push(exec::invalid_command(
            spu::MFC_GETLLAR,
            params,
            cellgov_dma::MfcCommandError::DataSegment { ea },
            state,
            unit,
        ));
        return MemoryReadCompletion::Refused;
    }
    effects.extend(shared_read(ea, size, unit));
    let Some(line_addr) = acquire_line else {
        return MemoryReadCompletion::Read { acquired: false };
    };
    // [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] the channel holds the status of the last completed immediate atomic command.
    state.channels.atomic_status = MFC_ATOMIC_STAT_G;
    state.channels.atomic_status_ready = true;
    state.set_reservation(Some(cellgov_sync::ReservedLine::containing(line_addr)));
    effects.push(Effect::ReservationAcquire {
        line_addr,
        source: unit,
    });
    MemoryReadCompletion::Read { acquired: true }
}
