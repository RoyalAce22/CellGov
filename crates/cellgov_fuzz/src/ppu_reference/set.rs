//! The vector-set form of the PPU reference: one architectural unit and
//! its named vectors, each replayed on its own.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::compare::replay_reference;
use super::types::{
    PpuReferenceArtifact, PpuReferenceError, PpuReferenceInputState, PpuReferenceObservation,
    PpuReferenceProvenance, PpuReferenceReplay, PPU_REFERENCE_SCHEMA_VERSION,
};
use super::units::{unit_key, word_unit, PpuReferenceUnit};
use super::validate::parse_reference_json;

/// Schema version of the vector-set form.
pub const PPU_REFERENCE_SET_SCHEMA_VERSION: u32 = 3;

/// Most vectors one set may hold.
pub const MAX_PPU_REFERENCE_VECTORS: usize = 64;

/// One reference file: the architectural unit and its vectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceSet {
    /// Version of this serialized schema.
    pub schema_version: u32,
    /// The architectural unit every vector exercises.
    pub unit: PpuReferenceUnit,
    /// The named vectors.
    pub vectors: Vec<PpuReferenceVector>,
}

/// One named vector: the words, the start state and memory, and the
/// expected end, as a single-vector artifact holds them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PpuReferenceVector {
    /// Name, unique in its set.
    pub name: String,
    /// Source of the expected end.
    pub provenance: PpuReferenceProvenance,
    /// Instruction words in execution order.
    pub words: Vec<u32>,
    /// Initial PPU state.
    pub initial_state: PpuReferenceInputState,
    /// Base address of `initial_memory`.
    pub memory_base: u64,
    /// Initial data bytes.
    pub initial_memory: Vec<u8>,
    /// Expected end.
    pub expected: PpuReferenceObservation,
}

impl PpuReferenceVector {
    /// The vector as a single-vector artifact of the current schema.
    pub fn to_artifact(&self) -> PpuReferenceArtifact {
        PpuReferenceArtifact {
            schema_version: PPU_REFERENCE_SCHEMA_VERSION,
            case_id: self.name.clone(),
            provenance: self.provenance.clone(),
            words: self.words.clone(),
            initial_state: self.initial_state.clone(),
            memory_base: self.memory_base,
            initial_memory: self.initial_memory.clone(),
            expected: self.expected.clone(),
        }
    }
}

/// A reference file in either form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuReferenceFile {
    /// One vector, the single-vector artifact.
    Single(Box<PpuReferenceArtifact>),
    /// One unit's vector set.
    Set(PpuReferenceSet),
}

/// One vector's replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuVectorReplay {
    /// The vector's name.
    pub name: String,
    /// What replaying it produced.
    pub replay: PpuReferenceReplay,
}

impl PpuReferenceReplay {
    /// Whether every execution path agrees with the others and with the
    /// reference.
    pub fn is_match(&self) -> bool {
        self.internal_divergence.is_none() && self.comparisons.iter().all(|c| c.is_match())
    }
}

/// Parses and validates a reference file. A file with a `unit` field is
/// a vector set; any other is a single-vector artifact. No reason in
/// either form may name a tracker issue.
///
/// # Errors
///
/// A file that does not parse, or that fails its form's checks.
pub fn parse_reference_file(json: &str) -> Result<PpuReferenceFile, PpuReferenceError> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    if value.get("unit").is_none() {
        let artifact = parse_reference_json(json)?;
        refuse_tracker_reason(&artifact.expected)?;
        return Ok(PpuReferenceFile::Single(Box::new(artifact)));
    }
    let set: PpuReferenceSet = serde_json::from_value(value)?;
    set.validate()?;
    Ok(PpuReferenceFile::Set(set))
}

impl PpuReferenceSet {
    /// Checks the set's version, its vectors and its reasons:
    ///
    /// - the schema version;
    /// - the vector count, and that each name is non-blank and unique;
    /// - each vector's artifact checks;
    /// - for an instruction, PPE-unimplemented or privileged unit, that
    ///   each vector holds a word of that unit;
    /// - that no reason names a tracker issue.
    ///
    /// # Errors
    ///
    /// The first check that fails.
    pub fn validate(&self) -> Result<(), PpuReferenceError> {
        if self.schema_version != PPU_REFERENCE_SET_SCHEMA_VERSION {
            return Err(PpuReferenceError::Version {
                found: self.schema_version,
                supported: PPU_REFERENCE_SET_SCHEMA_VERSION,
            });
        }
        if self.vectors.is_empty() || self.vectors.len() > MAX_PPU_REFERENCE_VECTORS {
            return Err(PpuReferenceError::VectorCount {
                found: self.vectors.len(),
                max: MAX_PPU_REFERENCE_VECTORS,
            });
        }
        let decodes = matches!(
            self.unit,
            PpuReferenceUnit::Instruction { .. }
                | PpuReferenceUnit::PpeUnimplemented
                | PpuReferenceUnit::Privileged
        );
        let mut names = BTreeSet::new();
        for vector in &self.vectors {
            if vector.name.trim().is_empty() || !names.insert(vector.name.as_str()) {
                return Err(PpuReferenceError::VectorName {
                    name: vector.name.clone(),
                });
            }
            vector.to_artifact().validate()?;
            if decodes
                && !vector
                    .words
                    .iter()
                    .any(|word| word_unit(*word).as_ref() == Some(&self.unit))
            {
                return Err(PpuReferenceError::VectorUnit {
                    name: vector.name.clone(),
                    unit: unit_key(&self.unit),
                });
            }
            refuse_tracker_reason(&vector.expected)?;
        }
        Ok(())
    }
}

/// Refuses an expected end with a reason that names a tracker issue.
fn refuse_tracker_reason(expected: &PpuReferenceObservation) -> Result<(), PpuReferenceError> {
    match tracker_reason(&serde_json::to_value(expected)?) {
        Some(reason) => Err(PpuReferenceError::TrackerReason { reason }),
        None => Ok(()),
    }
}

/// The first `reason` string under `value` that names a tracker issue:
/// `#` and a digit, a pointer that goes stale once its issue closes.
fn tracker_reason(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(fields) => fields.iter().find_map(|(key, field)| match field {
            serde_json::Value::String(text) if key == "reason" && names_tracker_issue(text) => {
                Some(text.clone())
            }
            _ => tracker_reason(field),
        }),
        serde_json::Value::Array(items) => items.iter().find_map(tracker_reason),
        _ => None,
    }
}

fn names_tracker_issue(text: &str) -> bool {
    text.split('#')
        .skip(1)
        .any(|after| after.starts_with(|c: char| c.is_ascii_digit()))
}

/// Replays every vector of a set.
///
/// # Errors
///
/// A set that fails its checks, or a vector the execution paths refuse.
pub fn replay_reference_set(
    set: &PpuReferenceSet,
) -> Result<Vec<PpuVectorReplay>, PpuReferenceError> {
    set.validate()?;
    set.vectors
        .iter()
        .map(|vector| {
            Ok(PpuVectorReplay {
                name: vector.name.clone(),
                replay: replay_reference(&vector.to_artifact())?,
            })
        })
        .collect()
}
