//! `dev ppu-coverage-gen`: render `docs/ppu_coverage.md` from the
//! tracked PPU evidence.
//!
//! `cellgov_fuzz::ppu_coverage` builds and renders the table; this
//! command gathers the evidence files it reads.

use std::path::{Path, PathBuf};

use cellgov_compare::hardware_capture::{profile_directories, OBSERVATION_FILE};
use cellgov_fuzz::ppu_census::{parse_census_record, PpuCensusRecordError};
use cellgov_fuzz::ppu_coverage::{
    console_case_units, coverage_rows, reference_files, render_coverage, PpuCoverageEvidence,
};
use cellgov_fuzz::ppu_reference::PpuReferenceCampaignError;
use cellgov_testkit::console_cases::{parse_table, CaseTableError};

use super::exit::{CommandError, CommandExitCode};
use super::parse::PpuCoverageGenArgs;

/// The PPU reference files, from the repository root.
const REFERENCE_DIR: &str = "crates/cellgov_fuzz/tests/fixtures/ppu_reference";

/// The microtest probes, from the repository root.
const PROBE_DIR: &str = "tests/micro";

/// The prefix of a PPU probe's directory name.
const PPU_PROBE_PREFIX: &str = "ppu_";

pub(crate) fn run(args: &PpuCoverageGenArgs) -> Result<CommandExitCode, CommandError> {
    let body = render(Path::new("."), &args.census)
        .map_err(|error| CommandError::failed(format!("ppu-coverage-gen: {error}")))?;
    std::fs::write(&args.output, body).map_err(|error| {
        CommandError::failed(format!(
            "ppu-coverage-gen: write {}: {error}",
            args.output.display()
        ))
    })?;
    println!("ppu-coverage-gen: wrote {}", args.output.display());
    Ok(CommandExitCode::SUCCESS)
}

/// An evidence file the table cannot be built from.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PpuCoverageGenError {
    /// A file or directory could not be read.
    #[error("{}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The census record does not parse.
    #[error("{}: {source}", path.display())]
    Census {
        path: PathBuf,
        #[source]
        source: PpuCensusRecordError,
    },
    /// The reference directory could not be read.
    #[error(transparent)]
    Reference(#[from] PpuReferenceCampaignError),
    /// A probe's case table does not parse.
    #[error("{}: {source}", path.display())]
    CaseTable {
        path: PathBuf,
        #[source]
        source: CaseTableError,
    },
}

fn read_error(path: &Path) -> impl FnOnce(std::io::Error) -> PpuCoverageGenError {
    let path = path.to_path_buf();
    move |source| PpuCoverageGenError::Read { path, source }
}

/// The coverage table from the evidence under `root`, with the census
/// record at `census` (relative to `root` when relative).
///
/// # Errors
///
/// Returns [`PpuCoverageGenError`] when an evidence file cannot be read
/// or does not parse.
pub(crate) fn render(root: &Path, census: &Path) -> Result<String, PpuCoverageGenError> {
    let census_path = root.join(census);
    let census_text = std::fs::read_to_string(&census_path).map_err(read_error(&census_path))?;
    let census =
        parse_census_record(&census_text).map_err(|source| PpuCoverageGenError::Census {
            path: census_path,
            source,
        })?;
    let reference_files = reference_files(&root.join(REFERENCE_DIR))?;
    let cases = captured_case_words(&root.join(PROBE_DIR))?;
    let evidence = PpuCoverageEvidence {
        reference_files,
        console_cases: console_case_units(cases.iter().map(Vec::as_slice)),
        census: Some(census),
    };
    let rows = coverage_rows(&evidence);
    Ok(render_coverage(&rows, evidence.census.as_ref()))
}

/// The words of every case of every PPU probe under `probes` that holds
/// a case table and a console capture.
fn captured_case_words(probes: &Path) -> Result<Vec<Vec<u32>>, PpuCoverageGenError> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(probes)
        .map_err(read_error(probes))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PPU_PROBE_PREFIX))
        })
        .collect();
    dirs.sort();
    let mut words = Vec::new();
    for dir in dirs {
        let table = dir.join("cases.tsv");
        if !table.is_file() || !has_capture(&dir) {
            continue;
        }
        let text = std::fs::read_to_string(&table).map_err(read_error(&table))?;
        let cases = parse_table(&text).map_err(|source| PpuCoverageGenError::CaseTable {
            path: table,
            source,
        })?;
        words.extend(cases.into_iter().map(|case| case.words));
    }
    Ok(words)
}

/// Whether some capture profile directory of the probe in `probe` holds
/// an observation.
fn has_capture(probe: &Path) -> bool {
    profile_directories(probe).is_ok_and(|profiles| {
        profiles
            .iter()
            .any(|profile| profile.join(OBSERVATION_FILE).is_file())
    })
}

#[cfg(test)]
#[path = "tests/ppu_coverage_gen_tests.rs"]
mod tests;
