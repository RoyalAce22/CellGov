//! The logical SPE hypercalls, with the isolated SPE calls of the SELF
//! decryption path.

pub(crate) mod construct;
mod irq_outlet;
mod registration;
pub(crate) mod shadow;

pub use construct::{LogicalSpe, LogicalSpeState};
pub use registration::{SpeChange, SpeState};
pub(crate) use registration::{GROUP, LOGICAL_SPE_SOURCE};
pub use shadow::{shadow_storage, SHADOW_STORAGE_BASE, SHADOW_STORAGE_PAGES};
