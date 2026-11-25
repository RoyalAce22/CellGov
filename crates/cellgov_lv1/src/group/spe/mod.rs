//! The logical SPE hypercalls, with the isolated SPE calls of the SELF
//! decryption path.

pub(crate) mod construct;
mod registration;

pub use construct::{LogicalSpe, LogicalSpeState};
pub use registration::{SpeChange, SpeState};
pub(crate) use registration::{GROUP, LOGICAL_SPE_SOURCE};
