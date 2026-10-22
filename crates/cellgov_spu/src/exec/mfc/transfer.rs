//! MFC transfers: the parameter checks and the DMA requests a put or
//! a get queues.

use crate::state::SpuState;
use cellgov_dma::{
    DmaDirection, DmaRequest, MfcCommandClass, MfcCommandError, MfcOrdering, MfcParameters,
};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::ppu::CELL_EA_LIMIT;
use cellgov_ps3_abi::hw::spu::MfcTagId;

use super::command::queue_invalid;
use crate::exec::outcome::SpuStepOutcome;

/// Checks a transfer's latched parameters against `class`, then queues it with `ordering`.
pub(super) fn issue_transfer(
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
    let request = local_store_transfer(direction, local, main, unit_id);
    let request = request
        .with_tag_id(tag)
        .with_ordering(ordering)
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

/// A transfer between `local` in the issuer's local store and `main`.
///
/// The runtime reads a put's source and writes its destination as the
/// put's bus requests complete, front to back. It lands a get when the
/// whole get completes. A store the issuer makes to a put's source while
/// the put is in flight reaches the part the MFC has not read yet. The
/// landing of a get replaces a store to its destination. A program
/// that orders its own accesses against a transfer waits for the tag
/// group first.
///
/// [CBEA p:173 s:10.3] the local-storage access of a queued command is complete when its tag group reads complete.
/// [SPU-ISA p:253 s:13] an external device's local-storage accesses can expose the SPU's reordering of its own, and the section names how to order the two.
pub(super) fn local_store_transfer(
    direction: DmaDirection,
    local: ByteRange,
    main: ByteRange,
    unit_id: UnitId,
) -> DmaRequest {
    match direction {
        DmaDirection::Put => DmaRequest::new(direction, local, main, unit_id)
            .expect("matching sizes")
            .with_local_store_source(),
        DmaDirection::Get => {
            DmaRequest::new(direction, main, local, unit_id).expect("matching sizes")
        }
    }
}

/// Checks a transfer's latched parameters against `class` and returns its tag group.
///
/// A command the MFC refuses still retires. It joins the queue as an
/// invalid command and takes a slot. The queue suspends when it reaches
/// that command. The `Err` is that step's outcome.
///
/// [CBEA p:113 s:9.1.1] the parameters' validity is checked asynchronous to the instruction stream.
pub(super) fn checked_transfer(
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
pub(super) fn latched_parameters(state: &SpuState) -> MfcParameters {
    let c = &state.channels;
    MfcParameters {
        lsa: c.mfc_lsa,
        eah: c.mfc_eah,
        eal: c.mfc_eal,
        size: c.mfc_size,
        tag: c.mfc_tag_id,
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
pub(super) fn atomic_segment_fault(ea: u64) -> Option<MfcCommandError> {
    (ea > CELL_EA_LIMIT).then_some(MfcCommandError::DataSegment { ea })
}

#[cfg(test)]
#[path = "tests/mfc_parameter_latch_tests.rs"]
mod mfc_parameter_latch_tests;
