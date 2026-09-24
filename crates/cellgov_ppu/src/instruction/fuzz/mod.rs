//! Defines interpreter-owned PPU instruction contracts for fuzzers.

mod bits;
mod classify;
mod descriptor;
mod fields;
mod metamorphic;
mod registry;
mod types;

pub use bits::{simplify_bit, simplify_encoding};
pub use registry::{expected_generation_kinds, generation_descriptor, generation_descriptors};
pub use types::*;

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/metamorphic_tests.rs"]
mod metamorphic_tests;
