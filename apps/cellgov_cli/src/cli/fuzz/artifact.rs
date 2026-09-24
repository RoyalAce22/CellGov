//! Independent reference checks, finding artifact storage, and exact replay.

use std::path::Path;

use cellgov_fuzz::artifact::{
    ArtifactReference, ArtifactReplayError, ArtifactStoreError, FuzzFindingArtifact,
};
use cellgov_fuzz::report::Finding;
use cellgov_fuzz::FuzzTarget;

use super::entry::write_stdout;
use super::error::FuzzCliError;
use super::outcome::{render_replay_outcome, ReplayOutcome};
use crate::cli::exit::CommandExitCode;
use crate::cli::parse::FuzzReplayArgs;

/// Reads the `--reference` file and holds it against the interpreter;
/// see [`ArtifactReference::checked`].
pub(super) fn read_reference(
    path: &Path,
    target: FuzzTarget,
) -> Result<ArtifactReference, FuzzCliError> {
    let json = std::fs::read_to_string(path).map_err(|source| FuzzCliError::ReferenceRead {
        path: path.to_path_buf(),
        source,
    })?;
    ArtifactReference::checked(&json, target).map_err(|error| match error {
        ArtifactReplayError::ReferenceMismatch => FuzzCliError::ReferenceMismatch,
        ArtifactReplayError::PpuReference(source) => FuzzCliError::PpuReference(source),
        ArtifactReplayError::SpuReference(source) => FuzzCliError::SpuReference(source),
        ArtifactReplayError::Artifact(source) => FuzzCliError::Artifact(source),
        other => FuzzCliError::ArtifactReplay(other),
    })
}

/// Stores `artifact` at `path`; see [`FuzzFindingArtifact::store`]. Every
/// refusal carries the artifact, so the error names its case even when no
/// file holds it.
pub(super) fn persist_finding(
    path: &Path,
    artifact: FuzzFindingArtifact,
) -> Result<(), FuzzCliError> {
    artifact
        .store(path)
        .map_err(|error| store_refusal(path, error, Box::new(artifact)))
}

/// The command's refusal for an artifact the store did not keep.
pub(super) fn store_refusal(
    path: &Path,
    error: ArtifactStoreError,
    artifact: Box<FuzzFindingArtifact>,
) -> FuzzCliError {
    let path = path.to_path_buf();
    match error {
        ArtifactStoreError::NoParent => FuzzCliError::Invalid("artifact path has no parent"),
        ArtifactStoreError::Encoding(source) => FuzzCliError::ArtifactEncoding { source, artifact },
        ArtifactStoreError::Write { source, .. } => FuzzCliError::ArtifactWrite {
            path,
            source,
            artifact,
        },
        ArtifactStoreError::Collision { .. } => FuzzCliError::ArtifactCollision { path, artifact },
        ArtifactStoreError::ReductionNotStored { stored, .. } => {
            FuzzCliError::ArtifactReductionNotStored {
                path,
                stored,
                artifact,
            }
        }
    }
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
