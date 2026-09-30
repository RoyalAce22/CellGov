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
// [SPU-ISA p:195 s:9.1] SPU single precision does not compute IEEE 754 results, so host floating point cannot stand in for it; cellgov_float computes it.
#![cfg_attr(not(test), forbid(clippy::float_arithmetic))]

pub mod census;
pub mod decode;
pub mod disasm;
pub mod exec;
mod fault_codes;
mod fpscr;
pub mod fuzz;
pub mod image;
pub mod instruction;
pub mod loader;
pub mod multilinear;
pub mod observation;
pub mod state;
pub mod stop;
mod unit;

pub use fault_codes::describe_guest_fault;
pub use unit::{SpuExecutionUnit, SpuSnapshot};

#[cfg(test)]
#[path = "tests/spu_tests.rs"]
mod tests;
