//! What one evaluation runs: engine, generator, budget, and every seed.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::reduce::ReductionRequest;
use crate::{
    CampaignSchedule, CampaignShard, CampaignVersion, CaseRange, ConfigurationError, FuzzConfig,
    FuzzTarget, GenerationStrategy, RetentionConfig, CAMPAIGN_VERSION,
};

// [Klees2018 p:2127 s:4 Statistically Sound Comparisons] A randomized fuzzer run once per side supports no ranking; both sides need many trials, and two is only the floor.
/// Fewest trials an evaluation runs; one trial gives no distribution.
pub const MIN_TRIALS: usize = 2;

/// Cases every trial attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialBudget {
    /// Case indices each trial considers, from index zero.
    pub cases: u64,
}

/// One engine and generator, run once per listed seed at one budget.
///
/// The plan fixes every seed before the first trial runs. A result reports
/// all of them, so a trial nobody liked cannot drop out of the sample.
/// [Klees2018 p:2128 s:4 Statistically Sound Comparisons]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationPlan {
    /// Generator version every trial replays under.
    pub campaign_version: CampaignVersion,
    /// Engine under evaluation.
    pub target: FuzzTarget,
    /// Generator under evaluation.
    pub strategy: GenerationStrategy,
    /// Budget shared by every trial.
    pub budget: TrialBudget,
    /// Trial seeds, in run order.
    pub seeds: Vec<u64>,
    /// Words per generated sequence case.
    pub sequence_words: u32,
    /// Findings each trial retains in detail.
    pub max_findings: u32,
    /// Case-retention settings each trial uses.
    pub retention: RetentionConfig,
    /// Reduction applied to each retained finding, when requested.
    pub reduction: Option<ReductionRequest>,
}

impl EvaluationPlan {
    /// A plan over `trials` consecutive seeds from `first_seed`.
    #[must_use]
    pub fn new(
        target: FuzzTarget,
        strategy: GenerationStrategy,
        cases: u64,
        first_seed: u64,
        trials: u32,
    ) -> Self {
        Self {
            campaign_version: CAMPAIGN_VERSION,
            target,
            strategy,
            budget: TrialBudget { cases },
            seeds: (0..trials)
                .map(|offset| first_seed.wrapping_add(u64::from(offset)))
                .collect(),
            sequence_words: 32,
            max_findings: 20,
            retention: RetentionConfig::default(),
            reduction: None,
        }
    }

    /// Checks the plan before any trial runs.
    ///
    /// # Errors
    ///
    /// Refuses fewer than [`MIN_TRIALS`] seeds, a repeated seed, an empty
    /// budget, or a campaign the engine would refuse.
    pub fn validate(&self) -> Result<(), EvaluationPlanError> {
        if self.seeds.len() < MIN_TRIALS {
            return Err(EvaluationPlanError::TooFewTrials {
                found: self.seeds.len(),
            });
        }
        let mut seen = BTreeSet::new();
        if let Some(&seed) = self.seeds.iter().find(|seed| !seen.insert(**seed)) {
            return Err(EvaluationPlanError::RepeatedSeed { seed });
        }
        if self.budget.cases == 0 {
            return Err(EvaluationPlanError::EmptyBudget);
        }
        self.trial_config(self.seeds[0])
            .validate_for_target(self.target)?;
        Ok(())
    }

    /// The campaign one trial runs.
    #[must_use]
    pub fn trial_config(&self, seed: u64) -> FuzzConfig {
        FuzzConfig {
            campaign_version: self.campaign_version,
            seed,
            strategy: self.strategy,
            schedule: CampaignSchedule {
                cases: CaseRange {
                    first: 0,
                    count: self.budget.cases,
                },
                shard: CampaignShard::ALL,
                cancellation: None,
            },
            retention: self.retention,
            max_findings: self.max_findings,
            sequence_words: self.sequence_words,
        }
    }
}

/// A plan no evaluation can run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvaluationPlanError {
    /// Too few seeds for a distribution.
    #[error("evaluation requires at least {MIN_TRIALS} trials, found {found}")]
    TooFewTrials {
        /// Seeds the plan lists.
        found: usize,
    },
    /// The same seed would run twice.
    #[error("evaluation lists seed {seed} more than once")]
    RepeatedSeed {
        /// Repeated seed.
        seed: u64,
    },
    /// No trial would consider a case.
    #[error("evaluation trial budget has no cases")]
    EmptyBudget,
    /// The engine refuses the trial campaign.
    #[error("evaluation campaign: {0}")]
    Configuration(#[from] ConfigurationError),
}
