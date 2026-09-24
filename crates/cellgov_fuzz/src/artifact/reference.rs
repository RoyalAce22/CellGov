//! The independent reference an artifact retains, and its replay check.

use serde::{Deserialize, Serialize};

use crate::ppu_reference::parse_reference_json as parse_ppu_reference;
use crate::spu_reference::parse_reference_json as parse_spu_reference;
use crate::FuzzTarget;

use super::finding::{ArtifactError, ArtifactReplayError};

/// Named independent source retained inside an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "source_json",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ArtifactReference {
    /// Checks only interpreter-owned local contracts.
    Local,
    /// Includes a validated PPU vector or hardware capture.
    Ppu(String),
    /// Includes a validated SPU vector or hardware capture.
    Spu(String),
}

impl ArtifactReference {
    /// Canonicalizes a versioned PPU vector or capture for portable replay.
    ///
    /// # Errors
    ///
    /// Refuses invalid source content or provenance.
    pub fn ppu(json: &str) -> Result<Self, ArtifactError> {
        Ok(Self::Ppu(serde_json::to_string(&parse_ppu_reference(
            json,
        )?)?))
    }

    /// Canonicalizes a versioned SPU vector or capture for portable replay.
    ///
    /// # Errors
    ///
    /// Refuses invalid source content or provenance.
    pub fn spu(json: &str) -> Result<Self, ArtifactError> {
        Ok(Self::Spu(serde_json::to_string(&parse_spu_reference(
            json,
        )?)?))
    }

    /// Parses an independent reference for `target`'s class, replays it
    /// against the interpreter, and canonicalizes it for portable replay.
    ///
    /// # Errors
    ///
    /// The reference's own parse or replay refusal, and
    /// [`ArtifactReplayError::ReferenceMismatch`] when any comparison
    /// differs from the interpreter.
    pub fn checked(json: &str, target: FuzzTarget) -> Result<Self, ArtifactReplayError> {
        let reference = match target {
            FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence => Self::Ppu(
                serde_json::to_string(&parse_ppu_reference(json)?).map_err(ArtifactError::from)?,
            ),
            FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence => Self::Spu(
                serde_json::to_string(&parse_spu_reference(json)?).map_err(ArtifactError::from)?,
            ),
        };
        reference.check()?;
        Ok(reference)
    }

    /// Replays this reference and refuses it unless it agrees with the
    /// interpreter on every comparison. A local reference names no
    /// independent source, so it holds.
    ///
    /// # Errors
    ///
    /// The reference's own parse or replay refusal, and
    /// [`ArtifactReplayError::ReferenceMismatch`].
    pub fn check(&self) -> Result<(), ArtifactReplayError> {
        match self {
            Self::Local => Ok(()),
            Self::Ppu(json) => {
                let source = parse_ppu_reference(json)?;
                let replay = crate::ppu_reference::replay_reference(&source)?;
                if replay.internal_divergence.is_some()
                    || replay.comparisons.is_empty()
                    || replay
                        .comparisons
                        .iter()
                        .any(|comparison| !comparison.is_match())
                {
                    return Err(ArtifactReplayError::ReferenceMismatch);
                }
                Ok(())
            }
            Self::Spu(json) => {
                let source = parse_spu_reference(json)?;
                let replay = crate::spu_reference::replay_reference(&source)?;
                if !replay.comparison.is_match() {
                    return Err(ArtifactReplayError::ReferenceMismatch);
                }
                Ok(())
            }
        }
    }

    pub(super) fn validate(&self, target: FuzzTarget) -> Result<(), ArtifactError> {
        match self {
            Self::Local => Ok(()),
            Self::Ppu(json)
                if matches!(target, FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence) =>
            {
                let source = parse_ppu_reference(json)?;
                (serde_json::to_string(&source)? == *json)
                    .then_some(())
                    .ok_or(ArtifactError::Invalid("PPU source is not canonical"))
            }
            Self::Spu(json)
                if matches!(target, FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence) =>
            {
                let source = parse_spu_reference(json)?;
                (serde_json::to_string(&source)? == *json)
                    .then_some(())
                    .ok_or(ArtifactError::Invalid("SPU source is not canonical"))
            }
            Self::Ppu(_) | Self::Spu(_) => Err(ArtifactError::Invalid(
                "reference interpreter differs from target",
            )),
        }
    }
}
