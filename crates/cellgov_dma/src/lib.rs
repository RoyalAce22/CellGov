//! cellgov_dma -- DMA request objects, queue model, modeled completions.
//!
//! DMA is not just memcpy. Represented here as `DmaRequest`, `DmaCompletion`,
//! and a `DmaLatencyModel` trait so the runtime seam is preserved against
//! later asynchronous backends. No actual platform async I/O lives here.

pub mod completion;
pub mod latency;
pub mod queue;
pub mod request;

pub use completion::DmaCompletion;
pub use latency::{DmaLatencyModel, FixedLatency};
pub use queue::DmaQueue;
pub use request::{DmaDirection, DmaRequest};
