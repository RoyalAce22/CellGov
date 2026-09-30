//! One campaign over a directory of SPU reference files: each file's
//! replay, and whether every architectural unit has exactly one file or
//! a pending entry.
//!
//! The units come from their sources: the SPU opcode map, the channel
//! map, the MFC command table and the facility list. A pending list in
//! the directory names each unit that has no file yet.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cellgov_ps3_abi::hw::spu::channel_direction;
use cellgov_ps3_abi::hw::spu_isa::{row_for, SPU_OPCODE_MAP};
use cellgov_ps3_abi::hw::spu_mfc::{MfcQueues, MFC_COMMANDS};

use super::compare::replay_reference;
use super::set_replay::{replay_reference_set, SpuVectorReplay};
use super::set_types::{SpuReferenceFacility, SpuReferenceFile, SpuReferenceUnit};
use super::set_validate::parse_reference_file;
use super::types::{SpuReferenceError, SpuReferenceReplay};

/// The file in a reference directory that lists the units with no file.
pub const SPU_REFERENCE_PENDING_FILE: &str = "pending.txt";

/// Channel numbers a channel instruction can name.
const CHANNEL_NUMBERS: u8 = 128;

/// The key a unit has in the pending list and the completeness report.
pub fn unit_key(unit: &SpuReferenceUnit) -> String {
    match unit {
        SpuReferenceUnit::Instruction { mnemonic } => format!("instruction:{mnemonic}"),
        SpuReferenceUnit::UnassignedOpcodes => "unassigned_opcodes".into(),
        SpuReferenceUnit::Channel { number } => format!("channel:{number}"),
        SpuReferenceUnit::ReservedChannels => "reserved_channels".into(),
        SpuReferenceUnit::MfcCommand { opcode } => format!("mfc_command:0x{opcode:04x}"),
        SpuReferenceUnit::OutsideSpuQueue => "outside_spu_queue".into(),
        SpuReferenceUnit::Facility { name } => format!("facility:{}", name.name()),
    }
}

/// Every unit a reference file can cover, in source order:
///
/// - every row of the SPU opcode map, then the unassigned words;
/// - every defined channel, then the reserved numbers;
/// - every MFC command the SPU queue accepts, then every other opcode;
/// - every facility.
pub fn all_units() -> Vec<SpuReferenceUnit> {
    let instructions = SPU_OPCODE_MAP
        .iter()
        .map(|row| SpuReferenceUnit::Instruction {
            mnemonic: row.mnemonic.into(),
        })
        .chain([SpuReferenceUnit::UnassignedOpcodes]);
    let channels = (0..CHANNEL_NUMBERS)
        .filter(|number| channel_direction(*number).is_some())
        .map(|number| SpuReferenceUnit::Channel { number })
        .chain([SpuReferenceUnit::ReservedChannels]);
    let commands = MFC_COMMANDS
        .iter()
        .filter(|command| command.queues != MfcQueues::ProxyOnly)
        .map(|command| SpuReferenceUnit::MfcCommand {
            opcode: command.opcode,
        })
        .chain([SpuReferenceUnit::OutsideSpuQueue]);
    let facilities = SpuReferenceFacility::ALL
        .into_iter()
        .map(|name| SpuReferenceUnit::Facility { name });
    instructions
        .chain(channels)
        .chain(commands)
        .chain(facilities)
        .collect()
}

/// The unit a parsed file covers. A single-vector file of one word
/// covers the opcode-map row its word selects; a single-vector file of
/// several words covers none.
pub fn file_unit(file: &SpuReferenceFile) -> Option<SpuReferenceUnit> {
    match file {
        SpuReferenceFile::Set(set) => Some(set.unit.clone()),
        SpuReferenceFile::Single(artifact) => match artifact.words.as_slice() {
            [word] => row_for(*word).map(|(_, row)| SpuReferenceUnit::Instruction {
                mnemonic: row.mnemonic.into(),
            }),
            _ => None,
        },
    }
}

/// What replaying one file produced.
#[derive(Debug)]
pub enum SpuReferenceFileOutcome {
    /// A single-vector file's replay.
    Single(Box<SpuReferenceReplay>),
    /// A vector set's replays, one per vector.
    Set(Vec<SpuVectorReplay>),
    /// The file did not parse or replay.
    Refused(SpuReferenceError),
}

impl SpuReferenceFileOutcome {
    /// Whether the file replayed and every vector matched.
    pub fn is_match(&self) -> bool {
        match self {
            Self::Single(replay) => replay.comparison.is_match(),
            Self::Set(replays) => replays.iter().all(|replay| replay.comparison.is_match()),
            Self::Refused(_) => false,
        }
    }
}

/// One file of the campaign.
#[derive(Debug)]
pub struct SpuReferenceFileRun {
    /// The file's name in the directory.
    pub file: String,
    /// The key of the unit it covers, or `None` for a file that covers
    /// none.
    pub unit: Option<String>,
    /// What replaying it produced.
    pub outcome: SpuReferenceFileOutcome,
}

/// Whether every unit has exactly one file or a pending entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpuReferenceCompleteness {
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

impl SpuReferenceCompleteness {
    /// Whether every unit has exactly one file or one pending entry, and
    /// every file and entry names a unit.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
            && self.duplicated.is_empty()
            && self.unowned.is_empty()
            && self.stale_pending.is_empty()
            && self.unknown_pending.is_empty()
    }
}

/// A campaign over one reference directory.
#[derive(Debug)]
pub struct SpuReferenceCampaign {
    /// Each JSON file, in name order.
    pub files: Vec<SpuReferenceFileRun>,
    /// The completeness verdict.
    pub completeness: SpuReferenceCompleteness,
}

impl SpuReferenceCampaign {
    /// Whether every file matched and the directory is complete.
    pub fn is_clean(&self) -> bool {
        self.completeness.is_complete() && self.files.iter().all(|run| run.outcome.is_match())
    }
}

/// A reference directory that cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum SpuReferenceCampaignError {
    /// A directory or file read failed.
    #[error("SPU reference read {}: {source}", path.display())]
    Read {
        /// The path.
        path: PathBuf,
        /// The failure.
        #[source]
        source: std::io::Error,
    },
}

fn read(path: &Path) -> Result<String, SpuReferenceCampaignError> {
    std::fs::read_to_string(path).map_err(|source| SpuReferenceCampaignError::Read {
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
) -> Result<SpuReferenceCampaign, SpuReferenceCampaignError> {
    let read_dir_error = |source| SpuReferenceCampaignError::Read {
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
        let (unit, outcome) = match parse_reference_file(&json) {
            Err(error) => (None, SpuReferenceFileOutcome::Refused(error)),
            Ok(file) => {
                let unit = file_unit(&file).map(|unit| unit_key(&unit));
                let outcome = match &file {
                    SpuReferenceFile::Single(artifact) => match replay_reference(artifact) {
                        Ok(replay) => SpuReferenceFileOutcome::Single(Box::new(replay)),
                        Err(error) => SpuReferenceFileOutcome::Refused(error),
                    },
                    SpuReferenceFile::Set(set) => match replay_reference_set(set) {
                        Ok(replays) => SpuReferenceFileOutcome::Set(replays),
                        Err(error) => SpuReferenceFileOutcome::Refused(error),
                    },
                };
                (unit, outcome)
            }
        };
        files.push(SpuReferenceFileRun {
            file: name,
            unit,
            outcome,
        });
    }
    let pending_path = dir.join(SPU_REFERENCE_PENDING_FILE);
    let pending = if pending_path.exists() {
        read(&pending_path)?
    } else {
        String::new()
    };
    let completeness = completeness(&files, &pending);
    Ok(SpuReferenceCampaign {
        files,
        completeness,
    })
}

/// Checks the files and the pending list's text against every unit.
pub fn completeness(files: &[SpuReferenceFileRun], pending: &str) -> SpuReferenceCompleteness {
    let units: BTreeSet<String> = all_units().iter().map(unit_key).collect();
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut report = SpuReferenceCompleteness {
        units: units.len(),
        ..SpuReferenceCompleteness::default()
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
