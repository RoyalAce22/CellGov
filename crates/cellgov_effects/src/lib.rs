//! cellgov_effects -- immutable effect packets emitted by execution units.
//!
//! Owns the `Effect` enum (`SharedWriteIntent`, `MailboxSend`, `MailboxReceiveAttempt`,
//! `DmaEnqueue`, `WaitOnEvent`, `WakeUnit`, `SignalUpdate`, `FaultRaised`, `TraceMarker`)
//! and the typed payload values they carry.
//!
//! This crate exists specifically so execution units do not depend on runtime
//! internals. Units produce `Effect` values; the runtime consumes them.
//!
//! `emitted_effects` ordering is preserved end-to-end: validation, conflict
//! diagnostics, fault attribution, and trace reconstruction all depend on
//! stable intra-step ordering even though commit batches are atomic.

pub mod effect;
pub mod payload;

pub use effect::Effect;
pub use payload::{FaultKind, MailboxMessage, WaitTarget, WritePayload};
