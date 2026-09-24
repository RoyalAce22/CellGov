//! Synergistic Processing Unit execution unit.
//!
//! Owns the fetch-decode-execute loop; instruction semantics live in
//! [`exec`], decoding in [`decode`]. Guest-visible writes flow through
//! `Effect` packets; reads into the 256 KB local store come from the
//! frozen committed view at [`cellgov_exec::ExecutionContext::memory`].

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

pub mod decode;
pub mod exec;
mod fault_codes;
pub mod fuzz;
pub mod instruction;
pub mod loader;
pub mod observation;
pub mod state;
mod unit;

pub use unit::{SpuExecutionUnit, SpuSnapshot};

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;
