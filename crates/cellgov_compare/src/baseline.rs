//! JSON save/load for `Observation` baselines on disk.

use crate::observation::Observation;
use std::io;
use std::path::{Path, PathBuf};

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

/// Why a directory of observations did not load.
#[derive(Debug, thiserror::Error)]
pub enum BaselineDirError {
    /// Listing the directory failed.
    #[error("failed to read observations directory {}: {source}", dir.display())]
    ReadDir {
        /// The directory.
        dir: PathBuf,
        /// The listing failure.
        #[source]
        source: io::Error,
    },
    /// Reading an entry of the directory failed.
    #[error("observations directory {}: failed to read entry: {source}", dir.display())]
    ReadEntry {
        /// The directory.
        dir: PathBuf,
        /// The entry failure.
        #[source]
        source: io::Error,
    },
    /// A `.json` file in the directory holds no observation.
    #[error("failed to load observation {}: {source}", path.display())]
    Load {
        /// The file.
        path: PathBuf,
        /// Why it did not load.
        #[source]
        source: BaselineError,
    },
}

/// Every `.json` observation in `dir`, sorted by file name, each with
/// the path it was read from.
pub fn load_dir(dir: &Path) -> Result<Vec<(PathBuf, Observation)>, BaselineDirError> {
    let entries = std::fs::read_dir(dir).map_err(|source| BaselineDirError::ReadDir {
        dir: dir.to_path_buf(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| BaselineDirError::ReadEntry {
            dir: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| match load(&path) {
            Ok(obs) => Ok((path, obs)),
            Err(source) => Err(BaselineDirError::Load { path, source }),
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/baseline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/baseline_dir_tests.rs"]
mod dir_tests;
