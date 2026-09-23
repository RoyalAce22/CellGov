//! Independent reference checks, finding artifact storage, and exact replay.

use std::io::Write;
use std::path::PathBuf;

use cellgov_fuzz::artifact::{
    ArtifactReduction, ArtifactReference, ArtifactReplayError, FuzzFindingArtifact,
};
use cellgov_fuzz::report::Finding;

use super::campaign::FuzzEngine;
use super::entry::write_stdout;
use super::error::FuzzCliError;
use super::outcome::{render_replay_outcome, ReplayOutcome};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::FuzzReplayArgs;

pub(super) fn check_reference(
    path: &PathBuf,
    engine: FuzzEngine,
) -> Result<ArtifactReference, FuzzCliError> {
    let json = std::fs::read_to_string(path).map_err(|source| FuzzCliError::ReferenceRead {
        path: path.clone(),
        source,
    })?;
    let reference = if engine.is_ppu() {
        let artifact = cellgov_fuzz::ppu_reference::parse_reference_json(&json)?;
        let replay = cellgov_fuzz::ppu_reference::replay_reference(&artifact)?;
        if replay.comparisons.is_empty()
            || replay.internal_divergence.is_some()
            || replay
                .comparisons
                .iter()
                .any(|comparison| !comparison.is_match())
        {
            return Err(FuzzCliError::ReferenceMismatch);
        }
        ArtifactReference::ppu(&json)?
    } else {
        let artifact = cellgov_fuzz::spu_reference::parse_reference_json(&json)?;
        let replay = cellgov_fuzz::spu_reference::replay_reference(&artifact)?;
        if !replay.comparison.is_match() {
            return Err(FuzzCliError::ReferenceMismatch);
        }
        ArtifactReference::spu(&json)?
    };
    Ok(reference)
}

pub(super) fn persist_finding(
    path: &PathBuf,
    artifact: FuzzFindingArtifact,
) -> Result<(), FuzzCliError> {
    let parent = path
        .parent()
        .ok_or(FuzzCliError::Invalid("artifact path has no parent"))?;
    std::fs::create_dir_all(parent).map_err(|source| FuzzCliError::ArtifactWrite {
        path: path.clone(),
        source,
        artifact: Box::new(artifact.clone()),
    })?;
    let encoded =
        serde_json::to_vec_pretty(&artifact).map_err(|source| FuzzCliError::ArtifactEncoding {
            source,
            artifact: Box::new(artifact.clone()),
        })?;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing =
                std::fs::read_to_string(path).map_err(|source| FuzzCliError::ArtifactWrite {
                    path: path.clone(),
                    source,
                    artifact: Box::new(artifact.clone()),
                })?;
            // A rerun with another `ArtifactExecutionPolicy` or campaign range
            // produces the same finding, so the first file stands. Only
            // different finding evidence collides. A finding's identity
            // excludes its reduction state, so a reduced rerun also matches
            // the stored file. The refusal carries the reduced case, which
            // no file holds.
            return match FuzzFindingArtifact::parse_json(&existing) {
                Ok(stored) if stored.describes_same_finding(&artifact) => {
                    if artifact.reduction != ArtifactReduction::NotAttempted
                        && stored.reduction != artifact.reduction
                    {
                        Err(FuzzCliError::ArtifactReductionNotStored {
                            path: path.clone(),
                            stored: stored.reduction,
                            artifact: Box::new(artifact),
                        })
                    } else {
                        Ok(())
                    }
                }
                _ => Err(FuzzCliError::ArtifactCollision {
                    path: path.clone(),
                    artifact: Box::new(artifact),
                }),
            };
        }
        Err(source) => {
            return Err(FuzzCliError::ArtifactWrite {
                path: path.clone(),
                source,
                artifact: Box::new(artifact),
            })
        }
    };
    file.write_all(&encoded)
        .and_then(|()| file.sync_all())
        .map_err(|source| FuzzCliError::ArtifactWrite {
            path: path.clone(),
            source,
            artifact: Box::new(artifact),
        })
}

pub(super) fn run_replay(args: &FuzzReplayArgs) -> Result<CommandExitCode, FuzzCliError> {
    run_replay_with(args, |artifact| {
        if args.reduced {
            artifact.replay_reduced()
        } else {
            artifact.replay()
        }
    })
}

pub(super) fn run_replay_with(
    args: &FuzzReplayArgs,
    replay: impl FnOnce(&FuzzFindingArtifact) -> Result<Finding, ArtifactReplayError>,
) -> Result<CommandExitCode, FuzzCliError> {
    let json =
        std::fs::read_to_string(&args.artifact).map_err(|source| FuzzCliError::ArtifactRead {
            path: args.artifact.clone(),
            source,
        })?;
    let artifact = FuzzFindingArtifact::parse_json(&json)?;
    let finding = replay(&artifact)?;
    let outcome = ReplayOutcome {
        case_index: finding.replay.case_index,
        reduced: args.reduced,
        finding_kind: format!("{:?}", finding.kind),
        fingerprint: artifact.fingerprint,
        words: finding.original_words,
    };
    write_stdout(&render_replay_outcome(&outcome))?;
    Ok(outcome.exit_code())
}
