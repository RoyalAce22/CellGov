//! Interpreter-owned contracts used by instruction fuzzers.

mod bits;
mod classify;
mod descriptor;
mod fields;
mod metamorphic;
mod registry;
mod relations;
mod sequence;
mod support;
mod types;

pub use bits::{shrink_instruction, simplify_instruction_bit};
pub use registry::{expected_generation_kinds, generation_descriptor, generation_descriptors};
pub use sequence::{SpuSequenceInteraction, REFUSED_MFC_OPCODE, SEQUENCE_MAILBOX_MESSAGE};
pub use support::{encoding_execution_is_supported, encoding_has_undefined_operands};
pub use types::*;

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/shufb_relation_tests.rs"]
mod shufb_relation_tests;

#[cfg(test)]
#[path = "tests/relation_catalog_tests.rs"]
mod relation_catalog_tests;

#[cfg(test)]
#[path = "tests/form_agreement_tests.rs"]
mod form_agreement_tests;
