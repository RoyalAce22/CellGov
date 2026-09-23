//! PPU fuzz engines built on interpreter-owned descriptors.
//!
//! The instruction engine and the sequence engine draw their cases from
//! `generate` and run them through `execute`. Each engine takes a case's
//! eligibility from `assess` and records what it finds through `record`.
//! `shrink` supplies the reduction candidates.

mod assess;
mod execute;
mod generate;
mod instructions;
mod record;
mod sequences;
mod shrink;

pub use instructions::run_instructions;
pub use sequences::run_sequences;

pub(crate) use instructions::{instruction_case_words, run_instructions_with};
pub(crate) use sequences::{run_sequences_with, sequence_case_words};
pub(crate) use shrink::{shrink_instruction_words, shrink_sequence_words};
