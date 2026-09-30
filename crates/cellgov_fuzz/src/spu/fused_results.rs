//! The check of an external fused form: a file of result states, each the
//! state a recompiler's fused form leaves from a start state the file names,
//! compared against sequence A of the row.
//!
//! [Martignoni2012 p:337 s:1] A test generated from the higher-fidelity
//! implementation runs on the lower-fidelity one, which then shows each
//! difference from the expected behavior. The file carries states and no
//! code, so nothing foreign runs.

use std::collections::BTreeMap;

use cellgov_spu::fuzz::SpuSequenceRelationId;
use cellgov_spu::observation::SpuObservationComponent;

use super::counterexample::{
    parse_lines, parse_registers, parse_row, state_of, CounterexampleError,
};
use super::sequence_relations::{compare_result_state, RelationInstance, RelationVerdict};
use crate::error::FuzzError;

/// Schema version of a result-state file.
pub(crate) const FUSED_RESULTS_SCHEMA_VERSION: u32 = 1;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultsJson {
    schema_version: u32,
    results: Vec<ResultJson>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultJson {
    name: String,
    row: String,
    assignment: Vec<u8>,
    registers: BTreeMap<String, String>,
    local_store: BTreeMap<String, String>,
    result: StateJson,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StateJson {
    registers: BTreeMap<String, String>,
    local_store: BTreeMap<String, String>,
    flow: String,
}

/// What the check found for one result state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FusedResultVerdict {
    /// The start state lies outside the row's precondition; nothing was
    /// compared.
    Inapplicable,
    /// The result state matches sequence A under the row's comparison.
    Match,
    /// The result state differs from sequence A.
    Diverged {
        /// The first component, in comparison order, that differs.
        first_component: SpuObservationComponent,
        /// The real registers that differ.
        registers: Vec<u8>,
    },
}

/// The check of one entry of a result-state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusedResult {
    /// The entry's name, unique within the file.
    pub name: String,
    /// The catalog row.
    pub relation: SpuSequenceRelationId,
    /// What the check found.
    pub verdict: FusedResultVerdict,
}

/// Why a result-state file was not checked.
#[derive(Debug, thiserror::Error)]
pub enum FusedResultsError {
    /// The text is not the file's JSON.
    #[error("result-state JSON does not parse: {source}")]
    Json {
        /// The parser's refusal.
        #[source]
        source: serde_json::Error,
    },
    /// The file names a schema this build does not read.
    #[error("result-state schema {found} is not {FUSED_RESULTS_SCHEMA_VERSION}")]
    Schema {
        /// The version the file names.
        found: u32,
    },
    /// An entry's row, assignment or state is out of form.
    #[error("result state {name}: {source}")]
    Entry {
        /// The entry's name.
        name: String,
        /// The field out of form.
        #[source]
        source: CounterexampleError,
    },
    /// An entry's flow is neither `FallThrough` nor `Taken`.
    #[error("result state {name} names flow {flow}, not FallThrough or Taken")]
    Flow {
        /// The entry's name.
        name: String,
        /// The flow the entry gives.
        flow: String,
    },
    /// Two entries share a name.
    #[error("result state name {name} appears twice in the file")]
    DuplicateName {
        /// The shared name.
        name: String,
    },
    /// Sequence A of a row did not run.
    #[error("result state {name} could not run its row: {source}")]
    Run {
        /// The entry's name.
        name: String,
        /// The runner's refusal.
        #[source]
        source: FuzzError,
    },
}

/// Checks every entry of the result-state file `text`, in file order.
///
/// # Errors
///
/// [`FusedResultsError`] names the first entry out of form, or the row that
/// that did not run; the check then compares no entry.
pub fn check_fused_results(text: &str) -> Result<Vec<FusedResult>, FusedResultsError> {
    let file: ResultsJson =
        serde_json::from_str(text).map_err(|source| FusedResultsError::Json { source })?;
    if file.schema_version != FUSED_RESULTS_SCHEMA_VERSION {
        return Err(FusedResultsError::Schema {
            found: file.schema_version,
        });
    }
    let mut names: Vec<String> = Vec::with_capacity(file.results.len());
    let mut parsed = Vec::with_capacity(file.results.len());
    for entry in file.results {
        if names.contains(&entry.name) {
            return Err(FusedResultsError::DuplicateName { name: entry.name });
        }
        names.push(entry.name.clone());
        let refused = |source| FusedResultsError::Entry {
            name: entry.name.clone(),
            source,
        };
        let relation = parse_row(&entry.name, &entry.row, &entry.assignment).map_err(refused)?;
        let start = state_of(
            &parse_registers(&entry.registers).map_err(refused)?,
            &parse_lines(&entry.local_store).map_err(refused)?,
        );
        let result = state_of(
            &parse_registers(&entry.result.registers).map_err(refused)?,
            &parse_lines(&entry.result.local_store).map_err(refused)?,
        );
        let taken = match entry.result.flow.as_str() {
            "FallThrough" => false,
            "Taken" => true,
            _ => {
                return Err(FusedResultsError::Flow {
                    name: entry.name,
                    flow: entry.result.flow,
                })
            }
        };
        let instance = RelationInstance {
            assignment: entry.assignment,
            start,
        };
        parsed.push((entry.name, relation, instance, result, taken));
    }
    parsed
        .into_iter()
        .map(|(name, relation, instance, result, taken)| {
            let verdict = match compare_result_state(relation, &instance, &result, taken) {
                Ok(RelationVerdict::Inapplicable) => FusedResultVerdict::Inapplicable,
                Ok(RelationVerdict::Match) => FusedResultVerdict::Match,
                Ok(RelationVerdict::Diverged(divergence)) => FusedResultVerdict::Diverged {
                    first_component: divergence.first_component,
                    registers: divergence
                        .bit_distance
                        .iter()
                        .map(|(register, _)| *register)
                        .collect(),
                },
                Err(source) => return Err(FusedResultsError::Run { name, source }),
            };
            Ok(FusedResult {
                name,
                relation: relation.id,
                verdict,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/fused_results_tests.rs"]
mod tests;
