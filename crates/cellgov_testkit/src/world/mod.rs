//! Fake [`ExecutionUnit`] implementations the scenario fixtures compose
//! to probe the runtime without an architectural interpreter.
//!
//! [`ExecutionUnit`]: cellgov_exec::ExecutionUnit

mod basic;
mod dma;
mod mailbox;
mod signal;
mod writers;

pub use basic::*;
pub use dma::*;
pub use mailbox::*;
pub use signal::*;
pub use writers::*;

#[cfg(test)]
#[path = "tests/world_tests.rs"]
mod tests;
