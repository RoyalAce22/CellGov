//! The channel reads and writes and the MFC command path.

use crate::state::SpuState;
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu;
use cellgov_ps3_abi::hw::spu::{MfcCmd, MfcTagId, MFC_ATOMIC_STAT_S, MFC_MAX_TAG_ID};
use cellgov_sync::RESERVATION_LINE_BYTES;
use cellgov_time::GuestTicks;

use super::outcome::{SpuFault, SpuStepOutcome};

pub(super) fn execute_wrch(
    channel: u8,
    rt: u8,
    state: &mut SpuState,
    unit_id: UnitId,
) -> SpuStepOutcome {
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
        spu::MFC_CMD => execute_mfc_cmd(val, state, unit_id),
        // [CBE-Handbook p:458 s:17. SPE Channel and Related MMIO Interface sub:17.10 MFC Tag-Group Management Channels] MFC_WrTagMask selects the tag groups included in subsequent tag-status queries.
        spu::MFC_WR_TAG_MASK => {
            state.channels.tag_mask = val;
            SpuStepOutcome::Continue
        }
        // [CBE-Handbook p:459 s:17. SPE Channel and Related MMIO Interface sub:17.10 MFC Tag-Group Management Channels] MFC_WrTagUpdate triggers when MFC_RdTagStat refreshes; immediate completion in this model.
        spu::MFC_WR_TAG_UPDATE => SpuStepOutcome::Continue,
        // [CBE-Handbook p:463 s:17. SPE Channel and Related MMIO Interface sub:17.12 SPU Mailbox Channels] SPU Write Outbound Mailbox sends a 32-bit message to the PPE; values are discarded here.
        spu::SPU_WR_OUT_MBOX => SpuStepOutcome::Continue,
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
    match channel {
        // [CBE-Handbook p:460 s:17. SPE Channel and Related MMIO Interface sub:17.10 MFC Tag-Group Management Channels] Read Tag-Group Status Channel: returns tag-status word; blocks until masked tags complete.
        spu::MFC_RD_TAG_STAT => {
            let masked = state.channels.tag_status & state.channels.tag_mask;
            if masked == state.channels.tag_mask {
                state.set_reg_word_splat(rt, state.channels.tag_status);
                SpuStepOutcome::Continue
            } else {
                SpuStepOutcome::Yield {
                    effects: vec![],
                    reason: YieldReason::DmaWait,
                }
            }
        }
        // [CBE-Handbook p:543 s:19. DMA Transfers and Interprocessor Communication sub:19.6 Mailboxes] SPU Read Inbound Mailbox is read-blocking when the mailbox is empty.
        spu::SPU_RD_IN_MBOX => {
            state.channels.pending_mbox_rt = Some(rt);
            SpuStepOutcome::Yield {
                effects: vec![Effect::MailboxReceiveAttempt {
                    mailbox: cellgov_sync::MailboxId::new(unit_id.raw()),
                    source: unit_id,
                }],
                reason: YieldReason::MailboxAccess,
            }
        }
        // [CBE-Handbook p:462 s:17. SPE Channel and Related MMIO Interface sub:17.11 MFC Read Atomic Command Status Channel] Reports success/failure status for the most recent atomic command (e.g. putllc).
        spu::MFC_RD_ATOMIC_STAT => {
            state.set_reg_word_splat(rt, state.channels.atomic_status);
            SpuStepOutcome::Continue
        }
        // [CBEA p:141 s:9.8 SPU Read Machine Status Channel] Two status bits: IS (bit 30) isolation and IE (bit 31) interrupt enable; the model runs nonisolated with interrupts never enabled, so both read as zero.
        spu::SPU_RD_MACH_STAT => {
            state.set_reg_word_splat(rt, 0);
            SpuStepOutcome::Continue
        }
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedChannel {
            channel,
            is_write: false,
        }),
    }
}

pub(super) fn execute_rchcnt(rt: u8, channel: u8, state: &mut SpuState) -> SpuStepOutcome {
    let count = match channel {
        // [CBEA p:141 s:9.8 SPU Read Machine Status Channel] The channel has no count; rchcnt on it always returns 1.
        spu::SPU_RD_MACH_STAT => 1,
        _ => return SpuStepOutcome::Fault(SpuFault::UnsupportedChannelCount(channel)),
    };
    state.regs[rt as usize] = [0u8; 16];
    state.set_reg_word_slot(rt, 0, count);
    SpuStepOutcome::Continue
}

fn execute_mfc_cmd(cmd: u32, state: &mut SpuState, unit_id: UnitId) -> SpuStepOutcome {
    // [CBE-Handbook p:456 s:17. SPE Channel and Related MMIO Interface sub:17.9 MFC Command Parameter Channels] A set bit above the tag field suspends MFC command queue processing, so no command naming that tag is processed.
    // The model has no suspended queue to hold the command in, and
    // carrying it would reach `1 << tag_id` on the completion path,
    // where a value past 31 has no bit to set. The command is refused
    // by name instead.
    if state.channels.mfc_tag_id > MFC_MAX_TAG_ID {
        return SpuStepOutcome::Fault(SpuFault::TagIdOutOfRange(state.channels.mfc_tag_id));
    }
    let word = MfcCmd::new(cmd);
    // [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] an invalid command suspends queue processing and raises an invalid-command interrupt, and the leading bit of the command halfword marks the opcode reserved.
    // The reserved bit outranks the low byte, so this check runs ahead
    // of the opcode match. A word that sets the bit names some other
    // command than its low byte spells.
    if word.names_a_reserved_opcode() {
        return SpuStepOutcome::Fault(SpuFault::UnsupportedMfcCommand(cmd));
    }
    // The two class ids ride in the same word and steer bus bandwidth
    // and cache replacement. They change how fast a command runs, never
    // what it does. This model has neither to steer, so it reads past
    // them.
    // [CBEA p:114 s:9.1.2 MFC Class ID Channel] a class id is never checked, an unrecognised one falls back to the default, and none of them raises an exception.
    let ea = ((state.channels.mfc_eah as u64) << 32) | state.channels.mfc_eal as u64;
    let lsa = state.channels.mfc_lsa;
    let size = state.channels.mfc_size;

    match word.opcode() {
        // [CBEA p:61 s:7. MFC Commands sub:7.6 Put Commands (Local Storage to Main Storage)] put: copy LS bytes to main storage.
        spu::MFC_PUT => {
            let lsa_usize = lsa as usize;
            let size_usize = size as usize;
            // MFC_LSA and MFC_Size arrive on separate channels and
            // neither write bounds the pair, so the source range is the
            // guest's to choose. The get side refuses the same shape
            // through `get_mut`. A direct index of local store here
            // panics the host on a range it cannot hold.
            let Some(ls_bytes) = lsa_usize
                .checked_add(size_usize)
                .and_then(|end| state.ls.get(lsa_usize..end))
                .map(|slice| slice.to_vec())
            else {
                return SpuStepOutcome::Fault(SpuFault::LsOutOfRange(lsa));
            };

            let src =
                ByteRange::new(GuestAddr::new(lsa as u64), size as u64).expect("valid LS range");
            let dst = ByteRange::new(GuestAddr::new(ea), size as u64).expect("valid EA range");
            let request = DmaRequest::new(DmaDirection::Put, src, dst, unit_id)
                .expect("matching sizes")
                // The gate at the top of this function refused anything
                // above the architected range, so the staged value is
                // inside it.
                .with_tag_id(
                    MfcTagId::new(state.channels.mfc_tag_id as u8)
                        .expect("invariant: the tag gate bounds the staged tag id"),
                );
            // [CBEA p:65 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] Self-store overlapping the reserved line clears the reservation.
            if let Some(line) = state.reservation {
                if line.overlaps_range(ea, size as u64) {
                    state.reservation = None;
                }
            }
            SpuStepOutcome::Yield {
                effects: vec![Effect::DmaEnqueue {
                    request,
                    payload: Some(ls_bytes),
                }],
                reason: YieldReason::DmaSubmitted,
            }
        }
        // [CBEA p:60 s:7. MFC Commands sub:7.5 Get Commands (Main Storage to Local Storage)] get: copy main-storage bytes into LS.
        spu::MFC_GET => {
            state.channels.pending_get = Some((ea, lsa, size, state.channels.mfc_tag_id as u8));
            SpuStepOutcome::Yield {
                effects: vec![],
                reason: YieldReason::DmaSubmitted,
            }
        }
        // [CBEA p:66 s:7.8.1 Get Lock Line and Reserve Command] getllar: the transfer is one cache line, placed in local storage, with a reservation over it.
        // The effective address names the line by any byte inside it.
        // [CBEA p:57 s:7.2 Command Exceptions] alignment is not checked for the atomic commands, so a misaligned address refuses nothing.
        // The bytes and the reservation therefore both cover the
        // containing line. The caller writes the reservation register
        // and the status channel after the line arrives; see
        // `SpuStepOutcome::MemoryRead`.
        spu::MFC_GETLLAR => {
            let line = cellgov_sync::ReservedLine::containing(ea);
            SpuStepOutcome::MemoryRead {
                ea: line.addr(),
                lsa,
                size: RESERVATION_LINE_BYTES as u32,
                acquire_line: Some(line.addr()),
            }
        }
        // [CBEA p:66 s:7. MFC Commands sub:7.8 MFC Atomic Update Commands] putllc: conditional store that succeeds only if the local reservation is still held for this line.
        spu::MFC_PUTLLC => {
            let line = cellgov_sync::ReservedLine::containing(ea);
            let success = match state.reservation {
                Some(l) => l.addr() == line.addr(),
                None => false,
            };
            if success {
                let lsa_usize = lsa as usize;
                // The line is 128 bytes wherever MFC_LSA points, and
                // nothing bounds that channel either.
                let Some(ls_bytes) = lsa_usize
                    .checked_add(RESERVATION_LINE_BYTES as usize)
                    .and_then(|end| state.ls.get(lsa_usize..end))
                    .map(|slice| slice.to_vec())
                else {
                    return SpuStepOutcome::Fault(SpuFault::LsOutOfRange(lsa));
                };
                state.reservation = None;
                // The store covers the line the reservation named, as
                // the getllar arm's read did.
                let range = ByteRange::new(GuestAddr::new(line.addr()), RESERVATION_LINE_BYTES)
                    .expect("valid EA range");
                state.channels.atomic_status = 0;
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
                SpuStepOutcome::Continue
            }
        }
        _ => SpuStepOutcome::Fault(SpuFault::UnsupportedMfcCommand(cmd)),
    }
}

#[cfg(test)]
#[path = "tests/mfc_class_id_tests.rs"]
mod mfc_class_id_tests;
