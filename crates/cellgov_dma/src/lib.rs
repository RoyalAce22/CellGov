//! DMA request/completion value types, a deterministic completion queue,
//! and a pluggable latency-model trait.

pub mod completion;
pub mod latency;
pub mod queue;
pub mod request;

pub use completion::DmaCompletion;
pub use latency::{DmaLatencyModel, FixedLatency};
pub use queue::DmaQueue;
pub use request::{DmaDirection, DmaRequest};
