//! Immutable effect packets emitted by execution units and consumed by the
//! commit pipeline.
//!
//! Exists so execution units do not depend on runtime internals: units
//! produce `Effect` values; the runtime consumes them.

pub mod effect;
pub mod payload;

pub use effect::Effect;
pub use payload::{FaultKind, MailboxMessage, WaitTarget, WritePayload};
