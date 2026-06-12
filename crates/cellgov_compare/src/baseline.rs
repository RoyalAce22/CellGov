//! JSON save/load for `Observation` baselines on disk.

use crate::observation::Observation;
use std::io;
use std::path::Path;

/// Why a baseline operation failed.
#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    /// File system error during save or load.
    #[error("baseline I/O: {0}")]
    Io(#[from] io::Error),
    /// JSON serialization or deserialization error.
    #[error("baseline JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Serialize `observation` as pretty-printed JSON to `path`.
pub fn save(observation: &Observation, path: &Path) -> Result<(), BaselineError> {
    let json = serde_json::to_string_pretty(observation)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Deserialize an observation from a JSON file at `path`.
pub fn load(path: &Path) -> Result<Observation, BaselineError> {
    let data = std::fs::read_to_string(path)?;
    let obs = serde_json::from_str(&data)?;
    Ok(obs)
}

#[cfg(test)]
#[path = "tests/baseline_tests.rs"]
mod tests;
