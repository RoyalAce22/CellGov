//! The interprocessor interrupt that lv1 sends to a running thread, and
//! the external interrupt that lv1's handler then raises for it.
//!
//! A raise on a plug of a running thread other than the caller stores
//! priority 0 in the interrupt generation port of that thread. The
//! thread takes the interrupt into lv1 at its next instruction boundary.
//! lv1's handler then tests the thread's status shadow against its mask;
//! see [`cellgov_ps3_abi::lv1::interrupt_mask`].

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_EE, MSR_HV};
use cellgov_ps3_abi::lv1::interrupt::THREADS_PER_LOGICAL_PPE;
use cellgov_ps3_abi::lv1::interrupt_mask::MEDIATED_UNDELIVERED_PC;

use crate::answer::UnmodelledError;
use crate::group::interrupt::bitmap::IrqStateBitmapKey;
use crate::group::interrupt::raise::unmasked;
use crate::group::interrupt::InterruptGroupState;
use crate::return_path::{external_raise, ExternalRaise};

/// The registers that lv1's external interrupt handler reads of a
/// running PPE hardware thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningThread {
    /// The address of the next instruction that the thread runs.
    pub pc: u64,
    /// The thread's MSR.
    pub msr: u64,
    /// The thread's decrementer at the time of the call.
    pub decrementer: u32,
}

/// The registers of each PPE hardware thread, by thread, or `None` for a
/// thread whose unit does not run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunningThreads(pub [Option<RunningThread>; THREADS_PER_LOGICAL_PPE as usize]);

impl RunningThreads {
    /// The registers of the thread `thread`, or `None`.
    pub fn of(&self, thread: u8) -> Option<RunningThread> {
        self.0.get(usize::from(thread)).copied().flatten()
    }
}

/// The external interrupt that lv1's handler raises for the running
/// thread `key`, or `None` when the handler raises none.
///
/// The handler reads the interrupt state `after`, the masks in `storage`
/// and the thread's registers `running`. It does these steps:
/// 1. It drains the thread's interrupt pending port, and tests the
///    decrementer [OFW-4.93 lv1 a:0x2C632C-0x2C6334].
/// 2. It tests the status shadow against the mask
///    [OFW-4.93 lv1 a:0x1A84-0x1AD4]. With no shared bit, it raises
///    nothing.
/// 3. With `MSR[EE]` = 0, it sets LPCR\[MER\] [OFW-4.93 lv1 a:0x187C]
///    [OFW-4.93 lv1 a:0x18D0-0x18DC]. That is [`ExternalRaise::Mediated`].
/// 4. Otherwise, it enters the vector with SRR0 = `running.pc` and
///    SRR1 = `running.msr`. That is [`ExternalRaise::Deliver`].
///
/// # Errors
///
/// Returns the refusal text for these inputs:
/// - `running` is `None`;
/// - the thread is in hypervisor state, where the handler makes no
///   shadow test [OFW-4.93 lv1 a:0x1A48-0x1A60];
/// - the decrementer is negative, which the handler sets to 0x7FFFFFFF
///   and then to 0 [OFW-4.93 lv1 a:0x2C636C-0x2C637C]
///   [OFW-4.93 lv1 a:0x1B88-0x1BA0];
/// - the shadow test refuses; see [`unmasked`];
/// - a set `MSR[EE]` and a pc of [`MEDIATED_UNDELIVERED_PC`], for which
///   the handler enters no vector [OFW-4.93 lv1 a:0x188C-0x18A8];
/// - [`external_raise`] refuses the MSR.
pub(crate) fn handler_raise(
    after: &InterruptGroupState,
    key: IrqStateBitmapKey,
    storage: &GuestMemory,
    running: Option<RunningThread>,
) -> Result<Option<ExternalRaise>, UnmodelledError> {
    let running = running.ok_or(UnmodelledError::new(
        "an interprocessor interrupt to a thread whose unit gives no running registers",
    ))?;
    if running.msr & MSR_HV != 0 {
        return Err(UnmodelledError::new(
            "an interprocessor interrupt to a thread in hypervisor state, for which lv1's \
             handler makes no shadow test",
        ));
    }
    if (running.decrementer as i32) < 0 {
        return Err(UnmodelledError::new(
            "an interprocessor interrupt to a thread whose decrementer is negative, which lv1's \
             handler resets",
        ));
    }
    if !unmasked(after.bitmaps(), key, storage)? {
        return Ok(None);
    }
    if running.msr & MSR_EE != 0 && running.pc == MEDIATED_UNDELIVERED_PC {
        return Err(UnmodelledError::new(
            "an interprocessor interrupt to a thread at lv1's trampoline slot 0x28B8, for which \
             lv1's handler enters no vector",
        ));
    }
    external_raise(running.msr).map(Some)
}

#[cfg(test)]
#[path = "tests/interprocessor_tests.rs"]
mod tests;
