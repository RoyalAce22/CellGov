//! MFC transfers into the SPU thread window: the effective addresses at
//! which LV2 maps each SPU thread of a group, its local store and then
//! its problem-state area.
//!
//! A transfer into the window of its issuer's own group reaches the
//! target unit instead of main storage. Each slot of the window holds
//! one thread's local store, then its problem-state registers. Of
//! those, a 4-byte put reaches the two signal-notification registers
//! and the inbound mailbox. Any other access into a slot is refused as
//! a data-storage fault.
//!
//! A transfer into the issuer's own slot is a local-store copy. Its
//! result is deterministic: a put writes the bytes it read at issue,
//! and a get reads the bytes the local store holds when it completes.
//!
//! [CBEA p:37 s:3.2] a local store can be aliased into the main storage domain.
//! [CBEA p:38 s:3.2.1] an MFC effective address can name an aliased local store, its own included.

use cellgov_dma::{DmaCompletion, DmaDirection, MfcCommandError};
use cellgov_event::UnitId;
use cellgov_exec::SignalNotifier;
use cellgov_lv2::thread_group::{ThreadGroupTable, MAX_SLOTS_PER_GROUP};
use cellgov_ps3_abi::hw::spu::{
    SPU_IN_MBOX_OFFSET, SPU_LS_SIZE, SPU_SIG_NOTIFY_1_OFFSET, SPU_SIG_NOTIFY_2_OFFSET,
};
use cellgov_ps3_abi::lv2::spu::thread_window;

/// Where a transfer into the SPU thread window lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowTarget {
    /// A range of `unit`'s local store, from `lsa`.
    LocalStore {
        /// The unit whose local store the transfer reaches.
        unit: UnitId,
        /// The local-store address of the transfer's first byte.
        lsa: u32,
    },
    /// One of `unit`'s signal-notification registers.
    Signal {
        /// The unit whose register the put writes.
        unit: UnitId,
        /// The register.
        register: SignalNotifier,
    },
    /// `unit`'s inbound mailbox.
    InboundMailbox {
        /// The unit whose mailbox the put writes.
        unit: UnitId,
    },
}

/// Where `c` lands in its issuer's group window, or `None` for a
/// transfer outside the window or an issuer in no thread group, which
/// resolves in main storage.
///
/// `Some(Err(_))` is a range in the window that names no thread or no
/// register the transfer can reach.
pub(super) fn window_target(
    groups: &ThreadGroupTable,
    c: &DmaCompletion,
) -> Option<Result<WindowTarget, MfcCommandError>> {
    let range = match c.direction() {
        DmaDirection::Put => c.destination(),
        DmaDirection::Get => c.source(),
    };
    let (ea, len) = (range.start().raw(), range.length());
    let end = thread_window::BASE + u64::from(MAX_SLOTS_PER_GROUP) * thread_window::STRIDE;
    if len == 0 || !(thread_window::BASE..end).contains(&ea) {
        return None;
    }
    // A transfer can complete after its issuer stops.
    let group = groups.group_of(c.issuer())?;
    let refused = Some(Err(MfcCommandError::DataStorage { ea }));
    let slot = (ea - thread_window::BASE) / thread_window::STRIDE;
    let offset = (ea - thread_window::BASE) % thread_window::STRIDE;
    // The slot is below MAX_SLOTS_PER_GROUP, so the thread id fits.
    let thread = group
        .checked_mul(MAX_SLOTS_PER_GROUP)
        .and_then(|base| base.checked_add(slot as u32));
    let Some(unit) = thread.and_then(|thread| groups.unit_for_thread(thread)) else {
        return refused;
    };
    if offset + len <= SPU_LS_SIZE as u64 {
        return Some(Ok(WindowTarget::LocalStore {
            unit,
            lsa: offset as u32,
        }));
    }
    if c.direction() != DmaDirection::Put || len != 4 {
        return refused;
    }
    let register = offset.checked_sub(thread_window::PROBLEM_STATE);
    Some(Ok(match register.map(|r| r as u32) {
        Some(SPU_SIG_NOTIFY_1_OFFSET) => WindowTarget::Signal {
            unit,
            register: SignalNotifier::One,
        },
        Some(SPU_SIG_NOTIFY_2_OFFSET) => WindowTarget::Signal {
            unit,
            register: SignalNotifier::Two,
        },
        Some(SPU_IN_MBOX_OFFSET) => WindowTarget::InboundMailbox { unit },
        _ => return refused,
    }))
}

#[cfg(test)]
#[path = "tests/spu_window_tests.rs"]
mod tests;
