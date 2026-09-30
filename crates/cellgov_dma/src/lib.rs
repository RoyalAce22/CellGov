//! DMA request/completion value types, a deterministic completion queue,
//! and a pluggable latency-model trait.

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

pub mod command;
pub mod completion;
pub mod latency;
pub mod queue;
pub mod request;

pub use command::{
    validate, InvalidMfcCommand, MfcCommandClass, MfcCommandError, MfcExceptionClass, MfcParameters,
};
pub use completion::DmaCompletion;
pub use latency::{DmaLatencyModel, FixedLatency};
pub use queue::{DmaQueue, DueCommands, RaisedMfcCommand};
pub use request::MfcOrdering;
pub use request::{DmaDirection, DmaRequest};
