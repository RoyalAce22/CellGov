//! The take of a mediated external interrupt: whether lv1's handler
//! enters the thread's vector with the registers that the PPU gave it.
//!
//! A thread takes the interrupt after a return set LPCR\[MER\] for it;
//! see [`crate::ExternalRaise::Mediated`]. lv1's handler then tests the
//! status shadow and the mask again; see
//! [`cellgov_ps3_abi::lv1::interrupt_mask`].

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu_interrupt::SRR1_INTERRUPT_INFO;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_HV, MSR_INTERRUPT_CLEARED};
use cellgov_ps3_abi::lv1::interrupt_mask::{
    EXTERNAL_INTERRUPT_MSR_CLEARED, MEDIATED_UNDELIVERED_PC,
};
use cellgov_ps3_abi::lv1::ppe::game_os_logical_ppe_id;

use crate::answer::{HypercallRefusal, UnmodelledError};
use crate::group::interrupt::bitmap::IrqStateBitmapKey;
use crate::group::interrupt::raise::unmasked;
use crate::group::interrupt::InterruptGroupState;

/// A thread's take of a mediated external interrupt, from the interrupt
/// record of its unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediatedTake {
    /// The unit that took the interrupt.
    pub unit: UnitId,
    /// The PPE hardware thread of the unit, or `None`.
    pub thread: Option<u8>,
    /// The SRR0 that the take set.
    pub srr0: u64,
    /// The SRR1 that the take set.
    pub srr1: u64,
}

/// The refusal of `take`, or `None` when lv1's handler enters the
/// vector with the registers of the take.
///
/// The test reads the interrupt state `interrupt` and the masks in
/// `storage`, the thread's real address space.
pub(crate) fn take_refusal(
    take: &MediatedTake,
    firmware: Option<&str>,
    interrupt: &InterruptGroupState,
    storage: &GuestMemory,
) -> Option<HypercallRefusal> {
    let error = check(take, firmware, interrupt, storage).err()?;
    Some(HypercallRefusal::MediatedExternal {
        unit: take.unit,
        pc: take.srr0,
        what: error.what,
    })
}

/// Checks `take` against the cases that the model covers.
///
/// # Errors
///
/// Returns the case of `take` that the model does not cover:
/// - No table gives the firmware's logical PPE id.
/// - The take's unit runs no PPE hardware thread.
/// - The shadow test of lv1's handler finds no bit
///   [OFW-4.93 lv1 a:0x1A84-0x1AD4]. The handler enters no vector.
/// - SRR0 is [`MEDIATED_UNDELIVERED_PC`]. The handler enters no vector.
/// - The MSR holds a bit of 48 to 63 that the PPU's interrupt entry
///   keeps. lv1 clears it in the vector's MSR [OFW-4.93 lv1 a:0x1898-0x18A4].
/// - The MSR holds HV. CellGov's partition never enters hypervisor state.
fn check(
    take: &MediatedTake,
    firmware: Option<&str>,
    interrupt: &InterruptGroupState,
    storage: &GuestMemory,
) -> Result<(), UnmodelledError> {
    let ppe_id = firmware
        .and_then(game_os_logical_ppe_id)
        .ok_or(UnmodelledError::new(
            "a take at a firmware whose logical PPE id no table gives",
        ))?;
    let thread = take.thread.ok_or(UnmodelledError::new(
        "a take on a unit that runs no PPE hardware thread",
    ))?;
    let key = IrqStateBitmapKey { ppe_id, thread };
    if !unmasked(interrupt.bitmaps(), key, storage)? {
        return Err(UnmodelledError::new(
            "a take whose status shadow and mask share no bit, for which lv1's handler enters no \
             vector",
        ));
    }
    if take.srr0 == MEDIATED_UNDELIVERED_PC {
        return Err(UnmodelledError::new(
            "a take at lv1's trampoline slot 0x28B8, for which lv1's handler enters no vector",
        ));
    }
    let msr = take.srr1 & !SRR1_INTERRUPT_INFO;
    if msr & EXTERNAL_INTERRUPT_MSR_CLEARED & !MSR_INTERRUPT_CLEARED != 0 {
        return Err(UnmodelledError::new(
            "a take whose MSR holds a bit of 48 to 63 that lv1 clears and the PPU's interrupt \
             entry keeps",
        ));
    }
    if msr & MSR_HV != 0 {
        return Err(UnmodelledError::new(
            "a take in hypervisor state, which lv1 treats as an interrupt of its own",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/mediated_tests.rs"]
mod tests;
