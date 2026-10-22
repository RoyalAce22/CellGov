//! MFC command decode: the opcode and class checks, the refusals, and
//! the command a write to MFC_Cmd queues.

use crate::state::SpuState;
use cellgov_dma::DmaDirection::{Get, Put};
use cellgov_dma::MfcCommandClass::{SendSignal, Transfer};
use cellgov_dma::MfcOrdering as Order;
use cellgov_dma::{InvalidMfcCommand, MfcCommandError, MfcParameters};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{BarrierKind, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::{MfcCmd, MFC_ATOMIC_STAT_S};
use cellgov_ps3_abi::hw::spu_mfc::{MfcOpcodeClass, MfcQueues};
use cellgov_sync::RESERVATION_LINE_BYTES;
use cellgov_time::GuestTicks;

use super::list::{held_list_orders, issue_list};
use super::ordering::{issue_ordering_command, issue_queued_lock_line, issue_storage_control};
use super::transfer::{atomic_segment_fault, issue_transfer, latched_parameters};
use crate::exec::channel::stall;
use crate::exec::outcome::{SpuFault, SpuStepOutcome};

/// The effect that queues `cmd` as a command the MFC refuses with
/// `error`. It takes a slot like any queued command.
pub(crate) fn invalid_command(
    cmd: u32,
    params: MfcParameters,
    error: MfcCommandError,
    state: &mut SpuState,
    unit_id: UnitId,
) -> Effect {
    // A held list that starts between steps finds the slot count the
    // last step left, which can be 0.
    state.channels.cmd_queue_free = state.channels.cmd_queue_free.saturating_sub(1);
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
pub(super) fn queue_invalid(
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
        // [CBEA p:68 s:7.8.4] putqlluc creates a tag-specific fence though it has no f modifier.
        spu::MFC_PUTQLLUC => BarrierKind::MfcFence,
        spu::MFC_PUTB | spu::MFC_PUTRB | spu::MFC_GETB | spu::MFC_SNDSIGB => {
            BarrierKind::MfcTagBarrier
        }
        spu::MFC_PUTLB | spu::MFC_PUTRLB | spu::MFC_GETLB => BarrierKind::MfcTagBarrier,
        spu::MFC_SDCRT | spu::MFC_SDCRTST | spu::MFC_SDCRZ => BarrierKind::MfcTagBarrier,
        spu::MFC_SDCRST | spu::MFC_SDCRF => BarrierKind::MfcTagBarrier,
        spu::MFC_SYNC => BarrierKind::MfcSync,
        spu::MFC_EIEIO => BarrierKind::MfcEieio,
        spu::MFC_BARRIER => BarrierKind::MfcBarrier,
        _ => return None,
    })
}

pub(in crate::exec) fn execute_mfc_cmd(
    cmd: u32,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
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
    if held_list_orders(cmd, state) {
        return stall();
    }
    // The two class ids ride in the same word and steer bus bandwidth
    // and cache replacement. They change how fast a command runs, never
    // what it does. This model has neither to steer, so it reads past
    // them.
    // [CBEA p:114 s:9.1.2 MFC Class ID Channel] a class id is never checked, an unrecognised one falls back to the default, and none of them raises an exception.
    let ea = ((state.channels.mfc_eah as u64) << 32) | state.channels.mfc_eal as u64;
    let lsa = state.channels.mfc_lsa;
    // [CBEA p:66 s:7.8.1] the getllar data transfer is one cache line.
    // No official page says which local-store line an unaligned MFC_LSA
    // names. The `spu_atomic_misaligned_lsa` console capture shows the
    // line that contains it, for getllar, putllc and putlluc, as for the
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
        // [CBE-Handbook p:151 s:6.2.2.4 Table 6-1] the CBE runs sdcrt and sdcrtst as nops.
        // [CBEA p:63 s:7.7.1] a touch is a hint, and it does not invoke the system error handler.
        // The model checks only their tag, as for the other commands that
        // move no bytes.
        spu::MFC_SDCRT | spu::MFC_SDCRTST => {
            issue_ordering_command(cmd, Order::TagBarrier, state, unit_id)
        }
        spu::MFC_SDCRZ => issue_storage_control(cmd, true, state, unit_id),
        // [CBEA p:64 s:7.7.4] sdcrst writes modified blocks to main storage; [CBEA p:64 s:7.7.5] sdcrf also invalidates them.
        // The `spu_own_put_reservation` console capture: neither loses a
        // reservation on the line, and neither raises the lost event.
        spu::MFC_SDCRST | spu::MFC_SDCRF => issue_storage_control(cmd, false, state, unit_id),
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
            let success = match state.reservation() {
                Some(l) => l.addr() == line.addr(),
                None => false,
            };
            if success {
                let ls_bytes = state.read_ls_wrapped(line_lsa, RESERVATION_LINE_BYTES as u32);
                state.set_reservation(None);
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
                state.set_reservation(None);
                state.channels.atomic_status = MFC_ATOMIC_STAT_S;
                state.channels.atomic_status_ready = true;
                SpuStepOutcome::Continue
            }
        }
        // [CBEA p:67 s:7.8.3] putlluc stores a cache line from local store whether or not a reservation exists.
        // [CBEA p:68 s:7.8.3] it is issued immediately, is not queued behind other commands, and has no tag.
        // The store covers the line that contains the effective address,
        // as getllar's read does, and the status channel reports U.
        spu::MFC_PUTLLUC => {
            if let Some(error) = atomic_segment_fault(ea) {
                return queue_invalid(cmd, error, state, unit_id);
            }
            let line = cellgov_sync::ReservedLine::containing(ea);
            let ls_bytes = state.read_ls_wrapped(line_lsa, RESERVATION_LINE_BYTES as u32);
            // [CBE-Handbook p:479 s:18.6.4] a matching putllc, putlluc or putqlluc resets the SPE's reservation, and raises no lock-line reservation lost event.
            if state
                .reservation()
                .is_some_and(|held| held.addr() == line.addr())
            {
                state.set_reservation(None);
            }
            let range = ByteRange::new(GuestAddr::new(line.addr()), RESERVATION_LINE_BYTES)
                .expect("valid EA range");
            state.channels.atomic_status = spu::MFC_ATOMIC_STAT_U;
            state.channels.atomic_status_ready = true;
            SpuStepOutcome::Yield {
                effects: vec![Effect::shared_write(
                    range,
                    WritePayload::new(ls_bytes),
                    unit_id,
                    GuestTicks::ZERO,
                )],
                reason: YieldReason::DmaSubmitted,
            }
        }
        spu::MFC_PUTQLLUC => issue_queued_lock_line(cmd, ea, line_lsa, state, unit_id),
        // Every command the SPU queue accepts has an arm above, so no
        // opcode reaches this one. The fault would mark a gap in the model.
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedMfcCommand(cmd)),
    }
}

#[cfg(test)]
#[path = "tests/mfc_class_id_tests.rs"]
mod mfc_class_id_tests;

#[cfg(test)]
#[path = "tests/mfc_opcode_class_tests.rs"]
mod mfc_opcode_class_tests;
