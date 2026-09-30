//! SPU fuzz engines built on interpreter-owned descriptors.
//!
//! The instruction engine and the sequence engine draw their cases from
//! `generate` and run them through `execute`. Each engine takes a case's
//! eligibility from `assess` and records what it finds through `record`.
//! `shrink` supplies the reduction candidates. The sequence engine also
//! checks one sequence relation per case through `sequence_relations`, and
//! `counterexample` keeps each relation finding's start state as a fixture
//! the engine replays first. `catalog` writes the rows as a reference, and
//! `fused_results` checks an external fused form's result states against
//! them.

mod assess;
mod catalog;
mod counterexample;
mod execute;
mod fused_results;
mod generate;
mod instructions;
mod record;
mod sequence_relations;
mod sequences;
mod shrink;

pub use catalog::{relation_catalog, RelationCatalog, CATALOG_JSON, CATALOG_MARKDOWN};
pub use counterexample::{CounterexampleError, CounterexampleStoreError, RelationCounterexample};
pub use fused_results::{check_fused_results, FusedResult, FusedResultVerdict, FusedResultsError};

pub(crate) use counterexample::counterexample_path;
pub use instructions::run_instructions;
pub use sequence_relations::{
    check_dead_sets, check_preconditions, measured_ulp, DeadSetFinding, PreconditionFinding,
};
pub use sequences::run_sequences;

pub(crate) use instructions::{instruction_case_words, run_instructions_with};
pub(crate) use sequences::{run_sequences_with, sequence_case_words};
pub(crate) use shrink::{shrink_instruction_words, shrink_sequence_words};
