//! The logical SPE hypercalls, with the isolated SPE calls of the SELF
//! decryption path.

pub(crate) mod construct;
mod interrupt_mask;
mod irq_outlet;
mod registration;
mod run_timeout;
pub(crate) mod shadow;
mod transition_notifier;

pub use construct::{LogicalSpe, LogicalSpeState};
pub use interrupt_mask::{InterruptMask, InterruptMaskState};
pub use registration::{SpeChange, SpeState};
pub(crate) use registration::{
    GROUP, LOGICAL_SPE_INTERRUPT_MASK_SOURCE, LOGICAL_SPE_NOTIFIER_SOURCE,
    LOGICAL_SPE_RUN_TIMEOUT_SOURCE, LOGICAL_SPE_SOURCE,
};
pub use run_timeout::{RunTimeout, RunTimeoutState};
pub use shadow::{shadow_storage, SHADOW_STORAGE_BASE, SHADOW_STORAGE_PAGES};
pub use transition_notifier::{TransitionNotifier, TransitionNotifierState};
