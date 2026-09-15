//! DMA request/completion value types, a deterministic completion queue,
//! and a pluggable latency-model trait.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod completion;
pub mod latency;
pub mod queue;
pub mod request;

pub use completion::DmaCompletion;
pub use latency::{DmaLatencyModel, FixedLatency};
pub use queue::DmaQueue;
pub use request::{DmaDirection, DmaRequest};
