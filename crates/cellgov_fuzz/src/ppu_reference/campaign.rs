//! One campaign over a directory of PPU reference files: each file's
//! replay, and whether every architectural unit has exactly one file or
//! a pending entry.
//!
//! The units come from the documented instruction listings and the
//! facility list. A pending list in the directory names each unit that
//! has no file yet.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_ppu::instruction::fuzz::{expected_generation_kinds, PpuFuzzKind};

use super::compare::replay_reference;
use super::set::{parse_reference_file, replay_reference_set, PpuReferenceFile, PpuVectorReplay};
use super::types::{PpuReferenceError, PpuReferenceReplay};
use super::units::{all_units, kind_unit, unit_key, word_unit, PpuReferenceUnit};

/// The file in a reference directory that lists the units with no file.
pub const PPU_REFERENCE_PENDING_FILE: &str = "pending.txt";

/// The unit a parsed file covers. A single-vector file of one word
/// covers that word's unit. A single-vector file covers no unit when it
/// holds several words or a word with no generation recipe.
pub fn file_unit(file: &PpuReferenceFile) -> Option<PpuReferenceUnit> {
    match file {
        PpuReferenceFile::Set(set) => Some(set.unit.clone()),
        PpuReferenceFile::Single(artifact) => match artifact.words.as_slice() {
            [word] => word_unit(*word),
            _ => None,
        },
    }
}

/// The decoded kinds whose unit is not among [`all_units`].
pub fn uncovered_kinds() -> Vec<PpuFuzzKind> {
    let units: BTreeSet<PpuReferenceUnit> = all_units().into_iter().collect();
    expected_generation_kinds()
        .into_iter()
        .filter(|kind| !units.contains(&kind_unit(*kind)))
        .collect()
}

/// What replaying one file produced.
#[derive(Debug)]
pub enum PpuReferenceFileOutcome {
    /// A single-vector file's replay.
    Single(Box<PpuReferenceReplay>),
    /// A vector set's replays, one per vector.
    Set(Vec<PpuVectorReplay>),
    /// The file did not parse or replay.
    Refused(PpuReferenceError),
}

impl PpuReferenceFileOutcome {
    /// Whether the file replayed and every vector matched.
    pub fn is_match(&self) -> bool {
        match self {
            Self::Single(replay) => replay.is_match(),
            Self::Set(replays) => replays.iter().all(|vector| vector.replay.is_match()),
            Self::Refused(_) => false,
        }
    }
}

/// One file of the campaign.
#[derive(Debug)]
pub struct PpuReferenceFileRun {
    /// The file's name in the directory.
    pub file: String,
    /// The key of the unit it covers, or `None` for a file that covers
    /// none.
    pub unit: Option<String>,
    /// What replaying it produced.
    pub outcome: PpuReferenceFileOutcome,
}

/// Whether every unit has exactly one file or a pending entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PpuReferenceCompleteness {
    /// Units a reference can cover.
    pub units: usize,
    /// Units with a file.
    pub covered: usize,
    /// Units the pending list names.
    pub pending: usize,
    /// Units with neither a file nor a pending entry.
    pub missing: Vec<String>,
    /// Units with more than one file, each with its files.
    pub duplicated: BTreeMap<String, Vec<String>>,
    /// Files that cover no unit.
    pub unowned: Vec<String>,
    /// Pending entries whose unit has a file.
    pub stale_pending: Vec<String>,
    /// Pending entries that name no unit, or name one twice.
    pub unknown_pending: Vec<String>,
}

impl PpuReferenceCompleteness {
    /// Whether every unit has exactly one file or one pending entry, and
    /// every file and entry names a unit.
    pub fn is_complete(&self) -> bool {
        self.gaps() == 0
    }

    /// Counts the gaps that [`Self::is_complete`] checks for.
    pub fn gaps(&self) -> usize {
        self.missing.len()
            + self.duplicated.len()
            + self.unowned.len()
            + self.stale_pending.len()
            + self.unknown_pending.len()
    }
}

/// A campaign over one reference directory.
#[derive(Debug)]
pub struct PpuReferenceCampaign {
    /// Each JSON file, in name order.
    pub files: Vec<PpuReferenceFileRun>,
    /// The completeness verdict.
    pub completeness: PpuReferenceCompleteness,
}

impl PpuReferenceCampaign {
    /// Whether every file matched and the directory is complete.
    pub fn is_clean(&self) -> bool {
        self.completeness.is_complete() && self.files.iter().all(|run| run.outcome.is_match())
    }

    /// Counts the vectors the files replayed. A refused file replays none.
    pub fn vectors(&self) -> usize {
        self.files
            .iter()
            .map(|run| match &run.outcome {
                PpuReferenceFileOutcome::Single(_) => 1,
                PpuReferenceFileOutcome::Set(replays) => replays.len(),
                PpuReferenceFileOutcome::Refused(_) => 0,
            })
            .sum()
    }
}

/// A reference directory that cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum PpuReferenceCampaignError {
    /// A directory or file read failed.
    #[error("PPU reference read {}: {source}", path.display())]
    Read {
        /// The path.
        path: PathBuf,
        /// The failure.
        #[source]
        source: std::io::Error,
    },
}

fn read(path: &Path) -> Result<String, PpuReferenceCampaignError> {
    std::fs::read_to_string(path).map_err(|source| PpuReferenceCampaignError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// Replays every `.json` file in `dir` and checks the directory against
/// every unit and its pending list.
///
/// # Errors
///
/// A directory, file or pending list that cannot be read.
pub fn run_reference_directory(
    dir: &Path,
) -> Result<PpuReferenceCampaign, PpuReferenceCampaignError> {
    let read_dir_error = |source| PpuReferenceCampaignError::Read {
        path: dir.to_path_buf(),
        source,
    };
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(read_dir_error)? {
        let name = entry.map_err(read_dir_error)?.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.ends_with(".json") {
            names.push(name);
        }
    }
    names.sort();
    let mut files = Vec::with_capacity(names.len());
    for name in names {
        let json = read(&dir.join(&name))?;
        let (unit, outcome) = replay_file(&json);
        files.push(PpuReferenceFileRun {
            file: name,
            unit,
            outcome,
        });
    }
    let pending_path = dir.join(PPU_REFERENCE_PENDING_FILE);
    let pending = if pending_path.exists() {
        read(&pending_path)?
    } else {
        String::new()
    };
    let completeness = completeness(&files, &pending);
    Ok(PpuReferenceCampaign {
        files,
        completeness,
    })
}

/// The unit key a file's text covers and what replaying it produced.
fn replay_file(json: &str) -> (Option<String>, PpuReferenceFileOutcome) {
    let file = match parse_reference_file(json) {
        Ok(file) => file,
        Err(error) => return (None, PpuReferenceFileOutcome::Refused(error)),
    };
    let unit = file_unit(&file).map(|unit| unit_key(&unit));
    let outcome = match &file {
        PpuReferenceFile::Single(artifact) => match replay_reference(artifact) {
            Ok(replay) => PpuReferenceFileOutcome::Single(Box::new(replay)),
            Err(error) => PpuReferenceFileOutcome::Refused(error),
        },
        PpuReferenceFile::Set(set) => match replay_reference_set(set) {
            Ok(replays) => PpuReferenceFileOutcome::Set(replays),
            Err(error) => PpuReferenceFileOutcome::Refused(error),
        },
    };
    (unit, outcome)
}

/// Checks the files and the pending list's text against every unit.
pub fn completeness(files: &[PpuReferenceFileRun], pending: &str) -> PpuReferenceCompleteness {
    let units: BTreeSet<String> = all_units().iter().map(unit_key).collect();
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut report = PpuReferenceCompleteness {
        units: units.len(),
        ..PpuReferenceCompleteness::default()
    };
    for run in files {
        match &run.unit {
            Some(unit) if units.contains(unit) => owners
                .entry(unit.clone())
                .or_default()
                .push(run.file.clone()),
            _ => report.unowned.push(run.file.clone()),
        }
    }
    let mut listed = BTreeSet::new();
    for entry in pending
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if !units.contains(entry) || !listed.insert(entry.to_string()) {
            report.unknown_pending.push(entry.to_string());
        } else if owners.contains_key(entry) {
            report.stale_pending.push(entry.to_string());
        }
    }
    report.covered = owners.len();
    report.pending = listed.len() - report.stale_pending.len();
    report.missing = units
        .iter()
        .filter(|unit| !owners.contains_key(*unit) && !listed.contains(*unit))
        .cloned()
        .collect();
    report.duplicated = owners
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .collect();
    report
}
