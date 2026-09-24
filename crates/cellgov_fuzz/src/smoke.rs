//! The bounded smoke set: the fixed campaigns every continuous build runs.
//!
//! One list names every campaign, its seed, its budget, and the coverage it
//! must reach; the command that runs the set and the tests that prove it
//! read the same list. A smoke campaign is deterministic and bounded, so a
//! continuous build runs it in seconds on every platform. The long runs
//! stay in the sweep, explicit and scheduled. A short run understates what
//! a fuzzer finds and can invert a comparison, so the smoke set gates only
//! determinism and coverage.
//! [Klees2018 p:2130 s:6 Timeouts]
//!
//! A bounded campaign that reaches nothing proves nothing. Every campaign
//! carries a coverage floor pinned from a measured run; a run under the floor
//! is a vacuous run and fails the set. The seeded-defect tests prove each
//! campaign still detects a defect at its budget: an injected defect is
//! ground truth, since the run knows which defect a finding hit.
//! [Klees2018 p:2132 s:7.1 Ground Truth: Bugs Found]

use crate::report::{FuzzReport, FuzzRun};
use crate::{
    CampaignSchedule, CampaignShard, CaseRange, FuzzConfig, FuzzTarget, GenerationStrategy,
    RetentionConfig, CAMPAIGN_VERSION,
};

/// Findings one smoke campaign retains in memory, enough to keep every class
/// its budget can reach.
const SMOKE_MAX_FINDINGS: u32 = 64;

/// Instruction words per sequence case. Pinned here rather than taken
/// from [`crate::DEFAULT_SEQUENCE_WORDS`]: the smoke campaigns' replay
/// coordinates, and so the regressions they match, must not move with a
/// caller's default.
const SMOKE_SEQUENCE_WORDS: u32 = 32;

/// The least coverage a smoke campaign must reach to count as a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageFloor {
    /// Cases that reached their check.
    pub eligible: u64,
    /// Distinct instruction kinds the campaign executed.
    pub instruction_kinds: u64,
}

/// One bounded campaign of the smoke set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmokeCampaign {
    /// Name the set, the command output, and the tests use.
    pub name: &'static str,
    /// Engine the campaign runs.
    pub target: FuzzTarget,
    /// How the campaign constructs input.
    pub strategy: GenerationStrategy,
    /// Master seed.
    pub seed: u64,
    /// Case indices the campaign considers, from zero.
    pub cases: u64,
    /// Words per case on a sequence engine.
    pub sequence_words: u32,
    /// Coverage the campaign must reach.
    pub floor: CoverageFloor,
}

/// Every campaign of the smoke set, in run order.
pub const SMOKE_CAMPAIGNS: [SmokeCampaign; 8] = [
    SmokeCampaign {
        name: "ppu-instruction-structured",
        target: FuzzTarget::PpuInstruction,
        strategy: GenerationStrategy::Structured,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 150,
            instruction_kinds: 40,
        },
    },
    SmokeCampaign {
        name: "ppu-instruction-raw-words",
        target: FuzzTarget::PpuInstruction,
        strategy: GenerationStrategy::RawWords,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 150,
            instruction_kinds: 40,
        },
    },
    SmokeCampaign {
        name: "ppu-sequence-structured",
        target: FuzzTarget::PpuSequence,
        strategy: GenerationStrategy::Structured,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 250,
            instruction_kinds: 40,
        },
    },
    SmokeCampaign {
        name: "ppu-sequence-raw-words",
        target: FuzzTarget::PpuSequence,
        strategy: GenerationStrategy::RawWords,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 250,
            instruction_kinds: 40,
        },
    },
    SmokeCampaign {
        name: "spu-instruction-structured",
        target: FuzzTarget::SpuInstruction,
        strategy: GenerationStrategy::Structured,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 200,
            instruction_kinds: 40,
        },
    },
    SmokeCampaign {
        name: "spu-instruction-raw-words",
        target: FuzzTarget::SpuInstruction,
        strategy: GenerationStrategy::RawWords,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 60,
            instruction_kinds: 20,
        },
    },
    SmokeCampaign {
        name: "spu-sequence-structured",
        target: FuzzTarget::SpuSequence,
        strategy: GenerationStrategy::Structured,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 200,
            instruction_kinds: 40,
        },
    },
    SmokeCampaign {
        name: "spu-sequence-raw-words",
        target: FuzzTarget::SpuSequence,
        strategy: GenerationStrategy::RawWords,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            eligible: 150,
            instruction_kinds: 40,
        },
    },
];

impl SmokeCampaign {
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
                    count: self.cases,
                },
                shard: CampaignShard::ALL,
                cancellation: None,
            },
            retention: RetentionConfig::default(),
            max_findings: SMOKE_MAX_FINDINGS,
            sequence_words: self.sequence_words,
        }
    }

    /// Runs the campaign through its engine.
    #[must_use]
    pub fn run(&self) -> FuzzRun {
        self.target.run(self.config())
    }

    /// Checks that a run of this campaign reached its coverage floor.
    ///
    /// # Errors
    ///
    /// Names the first metric under the floor.
    pub fn check_coverage(&self, report: &FuzzReport) -> Result<(), SmokeError> {
        let kinds = report.instruction_kinds.len() as u64;
        for (metric, found, floor) in [
            ("eligible", report.eligible_cases, self.floor.eligible),
            ("instruction_kinds", kinds, self.floor.instruction_kinds),
        ] {
            if found < floor {
                return Err(SmokeError::Vacuous {
                    name: self.name,
                    metric,
                    found,
                    floor,
                });
            }
        }
        Ok(())
    }
}

/// A smoke campaign whose run fails the set.
#[derive(Debug, thiserror::Error)]
pub enum SmokeError {
    /// The campaign reached less than its coverage floor.
    #[error("smoke campaign {name} reached {metric}={found}, under its floor of {floor}")]
    Vacuous {
        /// Campaign name.
        name: &'static str,
        /// Metric under the floor.
        metric: &'static str,
        /// Value the run reached.
        found: u64,
        /// Floor the campaign declares.
        floor: u64,
    },
}

#[cfg(test)]
#[path = "tests/smoke_tests.rs"]
mod tests;
