//! Promoted findings: the tracked, minimized regressions a finding becomes
//! before its fix lands.
//!
//! [`promote`] stores a finding's minimized artifact under a name in the
//! regression directory and lists it in the manifest as `open`. From then
//! on the tree carries the witness: the test that loads the directory
//! replays every entry and holds it to its status.
//!
//! - An open entry reproduces in the profile it names. A fix flips the
//!   entry to `fixed` in the same change; a fix that lands without the
//!   flip fails the entry as stale.
//! - A fixed entry, or an open entry in the other profile, does not
//!   reproduce. A defect that returns fails the entry as regressed.
//! - An unlisted artifact, or a listed name with no artifact, refuses the
//!   whole directory, so nothing skips.
//!
//! Identity is the artifact's semantic fingerprint and finding kind, never
//! its rendered text. [Chen2013 p:1 s:Abstract] The stored case is the
//! reducer's fixpoint, so the witness is the smallest case that keeps the
//! fingerprint. [Regehr2012 p:1 s:Abstract]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::artifact::{
    ArtifactFingerprint, ArtifactReduction, ArtifactReplayError, FuzzFindingArtifact,
};
use crate::report::Finding;

/// Schema version the regression manifest carries.
pub const REGRESSION_MANIFEST_VERSION: u32 = 1;

/// File name of the manifest inside a regression directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// The manifest: every promoted finding by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Every promoted finding, in the order the manifest lists them.
    pub regressions: Vec<RegressionEntry>,
}

/// One promoted finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegressionEntry {
    /// Name of the entry and of its artifact file, without the extension.
    pub name: String,
    /// Whether the defect is still in the tree.
    pub status: RegressionStatus,
    /// Build profile the finding reproduces in while open.
    pub profile: RegressionProfile,
    /// What the finding is, in one sentence.
    pub summary: String,
}

/// Whether a promoted finding's defect is still in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionStatus {
    /// The defect is in the tree; the artifact reproduces.
    Open,
    /// The defect is fixed; the artifact no longer reproduces.
    Fixed,
}

/// Build profile a promoted finding reproduces in while open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionProfile {
    /// Debug and release builds alike.
    Both,
    /// Debug builds only: the defect is a debug invariant.
    Debug,
    /// Release builds only.
    Release,
}

impl RegressionProfile {
    /// Whether this profile names the running build.
    #[must_use]
    pub fn matches_build(self) -> bool {
        match self {
            Self::Both => true,
            Self::Debug => cfg!(debug_assertions),
            Self::Release => !cfg!(debug_assertions),
        }
    }
}

/// One promoted finding with its artifact loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regression {
    /// The manifest entry.
    pub entry: RegressionEntry,
    /// The minimized artifact.
    pub artifact: FuzzFindingArtifact,
    /// Where the artifact was read from.
    pub path: PathBuf,
}

impl Regression {
    /// Whether the running build must reproduce this finding.
    #[must_use]
    pub fn expected_to_reproduce(&self) -> bool {
        self.entry.status == RegressionStatus::Open && self.entry.profile.matches_build()
    }

    /// Whether `finding` is this promoted finding: same engine, kind, and
    /// semantic fingerprint.
    #[must_use]
    pub fn covers(&self, finding: &Finding) -> bool {
        self.artifact.finding_kind == format!("{:?}", finding.kind)
            && self.artifact.fingerprint == ArtifactFingerprint::from(&finding.fingerprint)
    }

    /// Replays the artifact and holds it to the entry's expectation.
    ///
    /// # Errors
    ///
    /// - [`RegressionError::Stale`]: an open entry whose original case does
    ///   not reproduce.
    /// - [`RegressionError::ReducedLost`]: an open entry whose original case
    ///   reproduces but whose reduced case does not.
    /// - [`RegressionError::Regressed`]: a fixed entry, or an open entry of
    ///   another profile, that reproduces through either case.
    /// - [`RegressionError::Replay`]: a replay the engine could not finish.
    pub fn verify(&self) -> Result<(), RegressionError> {
        let expected = self.expected_to_reproduce();
        let original = self.replayed(self.artifact.replay())?;
        let reduced = match &self.artifact.reduction {
            ArtifactReduction::Reduced { .. } => {
                Some(self.replayed(self.artifact.replay_reduced())?)
            }
            _ => None,
        };
        let name = || self.entry.name.clone();
        if expected {
            if !original {
                return Err(RegressionError::Stale { name: name() });
            }
            if reduced == Some(false) {
                return Err(RegressionError::ReducedLost { name: name() });
            }
            Ok(())
        } else if original || reduced == Some(true) {
            Err(RegressionError::Regressed { name: name() })
        } else {
            Ok(())
        }
    }

    fn replayed(
        &self,
        replay: Result<Finding, ArtifactReplayError>,
    ) -> Result<bool, RegressionError> {
        match replay {
            Ok(_) => Ok(true),
            Err(ArtifactReplayError::NotReproduced { .. }) => Ok(false),
            Err(source) => Err(RegressionError::Replay {
                name: self.entry.name.clone(),
                source,
            }),
        }
    }
}

/// The promoted finding that covers `finding` in the running build, if any.
#[must_use]
pub fn promoted<'a>(regressions: &'a [Regression], finding: &Finding) -> Option<&'a Regression> {
    regressions
        .iter()
        .find(|regression| regression.expected_to_reproduce() && regression.covers(finding))
}

/// Promotes `artifact` into `dir` under `name` as an open regression.
///
/// The stored copy's replay command names the stored path, so it runs from
/// the regression directory. The manifest gains the entry last, after the
/// artifact is on disk.
///
/// # Errors
///
/// Refuses:
///
/// - a directory that does not load;
/// - a name in use, or one that is not a file stem;
/// - an artifact that is not minimized;
/// - a finding already promoted;
/// - an empty summary;
/// - a write the file system refused.
pub fn promote(
    dir: &Path,
    name: &str,
    profile: RegressionProfile,
    summary: &str,
    artifact: &FuzzFindingArtifact,
) -> Result<Regression, RegressionError> {
    let mut manifest = read_manifest(dir)?;
    let existing = load(dir)?;
    if !valid_name(name) {
        return Err(RegressionError::InvalidName {
            name: name.to_owned(),
        });
    }
    if existing.iter().any(|other| other.entry.name == name) {
        return Err(RegressionError::DuplicateName {
            name: name.to_owned(),
        });
    }
    if summary.trim().is_empty() {
        return Err(RegressionError::EmptySummary {
            name: name.to_owned(),
        });
    }
    if !matches!(
        artifact.reduction,
        ArtifactReduction::Reduced { .. } | ArtifactReduction::Irreducible
    ) {
        return Err(RegressionError::NotMinimized {
            name: name.to_owned(),
        });
    }
    if let Some(other) = existing.iter().find(|other| {
        other.artifact.finding_kind == artifact.finding_kind
            && other.artifact.fingerprint == artifact.fingerprint
    }) {
        return Err(RegressionError::DuplicateFinding {
            name: name.to_owned(),
            other: other.entry.name.clone(),
        });
    }
    let path = dir.join(format!("{name}.json"));
    let mut stored = artifact.clone();
    // The stored replay path is portable text: the directory as the caller
    // spelled it, every separator a forward slash, then the file. A forward
    // slash opens the file on every host.
    let replay_dir = dir
        .to_str()
        .ok_or(crate::artifact::ArtifactError::NonUtf8Path)
        .map_err(|source| RegressionError::Artifact {
            name: name.to_owned(),
            source,
        })?
        .replace('\\', "/");
    let replay_dir = replay_dir.trim_end_matches('/');
    if let Some(last) = stored.replay_command.last_mut() {
        *last = format!("{replay_dir}/{name}.json");
    }
    stored
        .validate()
        .map_err(|source| RegressionError::Artifact {
            name: name.to_owned(),
            source,
        })?;
    write_new(&path, &stored)?;
    let entry = RegressionEntry {
        name: name.to_owned(),
        status: RegressionStatus::Open,
        profile,
        summary: summary.trim().to_owned(),
    };
    manifest.regressions.push(entry.clone());
    write_new_or_replace(&dir.join(MANIFEST_FILE), &manifest)?;
    Ok(Regression {
        entry,
        artifact: stored,
        path,
    })
}

fn write_new<T: Serialize>(path: &Path, value: &T) -> Result<(), RegressionError> {
    let encoded = serde_json::to_vec_pretty(value).map_err(|source| RegressionError::Manifest {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(&encoded)?;
            file.write_all(b"\n")?;
            file.sync_all()
        })
        .map_err(|source| RegressionError::Write {
            path: path.to_path_buf(),
            source,
        })
}

fn write_new_or_replace<T: Serialize>(path: &Path, value: &T) -> Result<(), RegressionError> {
    let mut encoded =
        serde_json::to_vec_pretty(value).map_err(|source| RegressionError::Manifest {
            path: path.to_path_buf(),
            source,
        })?;
    encoded.push(b'\n');
    std::fs::write(path, encoded).map_err(|source| RegressionError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn read_manifest(dir: &Path) -> Result<RegressionManifest, RegressionError> {
    let manifest_path = dir.join(MANIFEST_FILE);
    let manifest = read(&manifest_path)?;
    let manifest: RegressionManifest =
        serde_json::from_str(&manifest).map_err(|source| RegressionError::Manifest {
            path: manifest_path.clone(),
            source,
        })?;
    if manifest.schema_version != REGRESSION_MANIFEST_VERSION {
        return Err(RegressionError::Version {
            found: manifest.schema_version,
            supported: REGRESSION_MANIFEST_VERSION,
        });
    }
    Ok(manifest)
}

/// Loads a regression directory: its manifest and every artifact it names.
///
/// # Errors
///
/// Refuses:
///
/// - a directory whose manifest and files disagree;
/// - an entry with no summary;
/// - an artifact that does not validate or is not minimized;
/// - two entries for one finding.
pub fn load(dir: &Path) -> Result<Vec<Regression>, RegressionError> {
    let manifest = read_manifest(dir)?;
    let mut names = BTreeSet::new();
    let mut regressions = Vec::with_capacity(manifest.regressions.len());
    for entry in manifest.regressions {
        if !valid_name(&entry.name) {
            return Err(RegressionError::InvalidName { name: entry.name });
        }
        if !names.insert(entry.name.clone()) {
            return Err(RegressionError::DuplicateName { name: entry.name });
        }
        if entry.summary.trim().is_empty() {
            return Err(RegressionError::EmptySummary { name: entry.name });
        }
        let path = dir.join(format!("{}.json", entry.name));
        let json = read(&path)?;
        let artifact =
            FuzzFindingArtifact::parse_json(&json).map_err(|source| RegressionError::Artifact {
                name: entry.name.clone(),
                source,
            })?;
        if !matches!(
            artifact.reduction,
            ArtifactReduction::Reduced { .. } | ArtifactReduction::Irreducible
        ) {
            return Err(RegressionError::NotMinimized { name: entry.name });
        }
        let regression = Regression {
            entry,
            artifact,
            path,
        };
        if let Some(other) = regressions.iter().find(|other: &&Regression| {
            other.artifact.finding_kind == regression.artifact.finding_kind
                && other.artifact.fingerprint == regression.artifact.fingerprint
        }) {
            return Err(RegressionError::DuplicateFinding {
                name: regression.entry.name,
                other: other.entry.name.clone(),
            });
        }
        regressions.push(regression);
    }
    if let Some(path) = stray_files(dir, &names)?.into_iter().next() {
        return Err(RegressionError::Stray { path });
    }
    Ok(regressions)
}

fn read(path: &Path) -> Result<String, RegressionError> {
    std::fs::read_to_string(path).map_err(|source| RegressionError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// A name is a file stem: lowercase ASCII letters, digits, `-` and `_`.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

/// Files in `dir` other than the manifest, a `README.md` and the listed
/// artifacts, sorted.
fn stray_files(dir: &Path, names: &BTreeSet<String>) -> Result<Vec<PathBuf>, RegressionError> {
    let entries = std::fs::read_dir(dir).map_err(|source| RegressionError::Read {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut stray = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RegressionError::Read {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let listed = file_name == MANIFEST_FILE
            || file_name == "README.md"
            || file_name
                .strip_suffix(".json")
                .is_some_and(|stem| names.contains(stem));
        if !listed {
            stray.push(path);
        }
    }
    stray.sort();
    Ok(stray)
}

/// A regression directory or entry that cannot stand as a witness.
#[derive(Debug, thiserror::Error)]
pub enum RegressionError {
    /// A file read failed.
    #[error("regression read {}: {source}", path.display())]
    Read {
        /// File or directory.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// The manifest does not parse, or a record does not encode.
    #[error("regression manifest {}: {source}", path.display())]
    Manifest {
        /// Manifest file.
        path: PathBuf,
        /// Why.
        #[source]
        source: serde_json::Error,
    },
    /// A file write failed.
    #[error("regression write {}: {source}", path.display())]
    Write {
        /// File.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// An entry or a promotion names no summary.
    #[error("regression {name} has an empty summary")]
    EmptySummary {
        /// Entry name.
        name: String,
    },
    /// The manifest schema is not the supported one.
    #[error("regression manifest schema {found} is not the supported {supported}")]
    Version {
        /// Version found.
        found: u32,
        /// Version supported.
        supported: u32,
    },
    /// An entry name is not a file stem.
    #[error("regression name {name:?} is not lowercase letters, digits, '-' and '_'")]
    InvalidName {
        /// The name.
        name: String,
    },
    /// Two entries share a name.
    #[error("regression {name} is listed twice")]
    DuplicateName {
        /// The name.
        name: String,
    },
    /// An artifact does not parse or validate.
    #[error("regression {name}: {source}")]
    Artifact {
        /// Entry name.
        name: String,
        /// Why.
        #[source]
        source: crate::artifact::ArtifactError,
    },
    /// An artifact was never reduced.
    #[error("regression {name} is not minimized: its artifact records no finished reduction")]
    NotMinimized {
        /// Entry name.
        name: String,
    },
    /// Two entries record one finding.
    #[error("regression {name} records the same finding as {other}")]
    DuplicateFinding {
        /// Entry name.
        name: String,
        /// The earlier entry.
        other: String,
    },
    /// A file in the directory that no entry names.
    #[error("regression directory holds {} that no entry names", path.display())]
    Stray {
        /// The file.
        path: PathBuf,
    },
    /// An open entry no longer reproduces: the fix landed without the flip.
    #[error("regression {name} is open but no longer reproduces; mark it fixed")]
    Stale {
        /// Entry name.
        name: String,
    },
    /// A fixed entry, or one of another profile, reproduces.
    #[error("regression {name} reproduces although the manifest says it must not")]
    Regressed {
        /// Entry name.
        name: String,
    },
    /// The original reproduces but the reduced case does not.
    #[error("regression {name}: the original reproduces but the reduced case lost the finding")]
    ReducedLost {
        /// Entry name.
        name: String,
    },
    /// The engine could not finish a replay.
    #[error("regression {name}: {source}")]
    Replay {
        /// Entry name.
        name: String,
        /// Why.
        #[source]
        source: ArtifactReplayError,
    },
}

#[cfg(test)]
#[path = "tests/regression_tests.rs"]
mod tests;
