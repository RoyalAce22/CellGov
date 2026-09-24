//! Replaying an artifact's original and reduced cases, and its independent-reference check.

use crate::reduce::evaluate_case;
use crate::report::{Finding, FuzzRun, RunOutcome};
use crate::{CampaignSchedule, CampaignShard, CaseRange, FuzzConfig};

use super::finding::{ArtifactReplayError, FuzzFindingArtifact};
use super::schema::{ArtifactFingerprint, ArtifactObservation, ArtifactReduction};

impl FuzzFindingArtifact {
    /// Replays this artifact through the selected interpreter engine.
    ///
    /// # Errors
    ///
    /// Refuses incompatible versions or any changed original fingerprint.
    pub fn replay(&self) -> Result<Finding, ArtifactReplayError> {
        self.replay_with(|config| self.original.replay.target.run(config))
    }

    /// Replays through a caller-supplied runner, behind the validation gate of [`Self::replay`].
    ///
    /// # Errors
    ///
    /// Refuses incompatible versions or a missing original fingerprint.
    pub fn replay_with(
        &self,
        run: impl FnOnce(FuzzConfig) -> FuzzRun,
    ) -> Result<Finding, ArtifactReplayError> {
        self.validate()?;
        self.check_independent_reference()?;
        let mut config = self.campaign;
        config.schedule = CampaignSchedule {
            cases: CaseRange {
                first: self.original.replay.case_index,
                count: 1,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        };
        let replay = run(config);
        if let RunOutcome::HarnessFailure(source) = replay.outcome {
            return Err(ArtifactReplayError::HarnessFailure { source });
        }
        // [Regehr2012 p:10 s:7.7 When Does Reduction Fail?] A crash's identifying
        // string tells one defect from another.
        // A target panic with a changed message is a different finding, so the payload compares
        // like the fingerprint. Engines never reduce, so the comparison skips the reduction and a
        // reduced artifact still replays its original case.
        for finding in replay.report.findings {
            if finding.replay == self.original.replay
                && finding.original_words == self.original.words
                && ArtifactFingerprint::from(&finding.fingerprint) == self.fingerprint
                && format!("{:?}", finding.kind) == self.finding_kind
                && finding.observation.as_ref().map(ArtifactObservation::from) == self.observation
                && finding.panic_payload == self.panic_payload
            {
                return Ok(finding);
            }
        }
        Err(ArtifactReplayError::NotReproduced {
            case_index: self.original.replay.case_index,
        })
    }

    /// Replays the reduced case words and requires the recorded finding identity.
    ///
    /// A reduced case keeps the original's generated state, so its observation
    /// may differ. Its kind, fingerprint and panic payload stay the same.
    ///
    /// # Errors
    ///
    /// Refuses:
    ///
    /// - an artifact without a reduced case
    /// - an incompatible version
    /// - an engine failure
    /// - reduced words that no longer reproduce the finding
    pub fn replay_reduced(&self) -> Result<Finding, ArtifactReplayError> {
        self.validate()?;
        let ArtifactReduction::Reduced { words } = &self.reduction else {
            return Err(ArtifactReplayError::NoReducedCase);
        };
        self.check_independent_reference()?;
        let replay = evaluate_case(
            self.original.replay.target,
            self.campaign,
            self.original.replay.case_index,
            words,
        );
        self.reduced_finding(replay)
    }

    /// Finds the recorded finding identity in a reduced-case run.
    pub(super) fn reduced_finding(&self, replay: FuzzRun) -> Result<Finding, ArtifactReplayError> {
        if let RunOutcome::HarnessFailure(source) = replay.outcome {
            return Err(ArtifactReplayError::HarnessFailure { source });
        }
        replay
            .report
            .findings
            .into_iter()
            .find(|finding| {
                finding.replay == self.original.replay
                    && ArtifactFingerprint::from(&finding.fingerprint) == self.fingerprint
                    && format!("{:?}", finding.kind) == self.finding_kind
                    && finding.panic_payload == self.panic_payload
            })
            .ok_or(ArtifactReplayError::NotReproduced {
                case_index: self.original.replay.case_index,
            })
    }

    fn check_independent_reference(&self) -> Result<(), ArtifactReplayError> {
        self.reference.check()
    }
}
