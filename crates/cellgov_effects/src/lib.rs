//! Immutable effect packets emitted by execution units and consumed by the
//! commit pipeline.
//!
//! Exists so execution units do not depend on runtime internals: units
//! produce `Effect` values; the runtime consumes them.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod effect;
pub mod payload;

pub use effect::{Effect, EffectKind};
pub use payload::{FaultKind, MailboxMessage, WaitTarget, WritePayload};
