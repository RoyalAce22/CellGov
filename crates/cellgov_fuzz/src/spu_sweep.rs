//! The long SPU sweep: the campaigns the scheduled fuzz workflow runs over
//! every SPU engine and strategy.
//!
//! A campaign's budget is a count of cases per registry kind. The sweep
//! then grows with the registry, and each kind keeps the same expected
//! number of draws. A sweep run must reach at least the floor of the smoke
//! campaign with its engine and strategy. Its seeds differ from the smoke
//! set's, so a sweep reaches cases the smoke set never runs.

use crate::registry::spu_descriptors;
use crate::report::FuzzRun;
use crate::smoke::{SmokeCampaign, SMOKE_CAMPAIGNS};
use crate::{
    CampaignSchedule, CampaignShard, CaseRange, FuzzConfig, FuzzTarget, GenerationStrategy,
    RetentionConfig, CAMPAIGN_VERSION,
};

/// Findings one sweep campaign retains in memory.
const SWEEP_MAX_FINDINGS: u32 = 64;

/// Instruction words per sequence case, as in the smoke set.
const SWEEP_SEQUENCE_WORDS: u32 = 32;

/// One campaign of the long SPU sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepCampaign {
    /// Name the workflow output and the tests use.
    pub name: &'static str,
    /// Engine the campaign runs.
    pub target: FuzzTarget,
    /// How the campaign constructs input.
    pub strategy: GenerationStrategy,
    /// Master seed.
    pub seed: u64,
    /// Cases per kind of the SPU registry.
    pub cases_per_kind: u64,
    /// Words per case on a sequence engine.
    pub sequence_words: u32,
}

/// Every campaign of the long SPU sweep, in run order.
pub const SPU_SWEEP: [SweepCampaign; 4] = [
    SweepCampaign {
        name: "spu-instruction-structured-sweep",
        target: FuzzTarget::SpuInstruction,
        strategy: GenerationStrategy::Structured,
        seed: 2,
        cases_per_kind: 1024,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
    SweepCampaign {
        name: "spu-instruction-raw-words-sweep",
        target: FuzzTarget::SpuInstruction,
        strategy: GenerationStrategy::RawWords,
        seed: 2,
        cases_per_kind: 1024,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
    SweepCampaign {
        name: "spu-sequence-structured-sweep",
        target: FuzzTarget::SpuSequence,
        strategy: GenerationStrategy::Structured,
        seed: 2,
        cases_per_kind: 256,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
    SweepCampaign {
        name: "spu-sequence-raw-words-sweep",
        target: FuzzTarget::SpuSequence,
        strategy: GenerationStrategy::RawWords,
        seed: 2,
        cases_per_kind: 256,
        sequence_words: SWEEP_SEQUENCE_WORDS,
    },
];

impl SweepCampaign {
    /// Case indices the campaign considers: its per-kind budget times the
    /// SPU registry size.
    #[must_use]
    pub fn cases(&self) -> u64 {
        spu_descriptors().len() as u64 * self.cases_per_kind
    }

    /// The configuration this campaign runs under.
    #[must_use]
    pub fn config(&self) -> FuzzConfig {
        FuzzConfig {
            campaign_version: CAMPAIGN_VERSION,
            seed: self.seed,
            strategy: self.strategy,
            schedule: CampaignSchedule {
                cases: CaseRange {
                    first: 0,
                    count: self.cases(),
                },
                shard: CampaignShard::ALL,
                cancellation: None,
            },
            retention: RetentionConfig::default(),
            max_findings: SWEEP_MAX_FINDINGS,
            sequence_words: self.sequence_words,
        }
    }

    /// Runs the campaign through its engine.
    #[must_use]
    pub fn run(&self) -> FuzzRun {
        self.target.run(self.config())
    }

    /// The smoke campaign with this engine and strategy, whose floor a
    /// sweep run must also reach.
    #[must_use]
    pub fn smoke(&self) -> Option<&'static SmokeCampaign> {
        SMOKE_CAMPAIGNS
            .iter()
            .find(|campaign| campaign.target == self.target && campaign.strategy == self.strategy)
    }
}

#[cfg(test)]
#[path = "tests/spu_sweep_tests.rs"]
mod tests;
