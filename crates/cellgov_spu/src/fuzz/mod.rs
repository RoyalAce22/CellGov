//! Interpreter-owned contracts used by instruction fuzzers.

mod bits;
mod classify;
mod descriptor;
mod fields;
mod metamorphic;
mod registry;
mod sequence;
mod support;
mod types;

pub use bits::{shrink_instruction, simplify_instruction_bit};
pub use registry::{expected_generation_kinds, generation_descriptor, generation_descriptors};
pub use sequence::SpuSequenceInteraction;
pub use support::{encoding_execution_is_supported, encoding_has_undefined_operands};
pub use types::*;

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;
