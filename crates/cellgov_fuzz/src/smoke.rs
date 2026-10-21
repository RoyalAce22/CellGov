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
//!
//! Each floor also names the registry size of the run that measured it. A
//! registry that grows or shrinks fails the set until a run measures the
//! floor again. The SPU part of the smoke set has five tiers:
//!
//! - the structured instruction campaign;
//! - the raw-word instruction campaign;
//! - the sequence campaigns against the scripted world;
//! - the metamorphic relations those campaigns check;
//! - the replay of the committed reference fixtures ([`SMOKE_REFERENCE`]).

use std::path::Path;

use cellgov_spu::fuzz::sequence_relations;

use crate::registry::{ppu_descriptors, spu_descriptors};
use crate::report::{CheckIdentity, FuzzReport, FuzzRun, InstructionIdentity};
use crate::spu_reference::{
    all_units, run_reference_directory, SpuReferenceCampaign, SpuReferenceCampaignError,
};
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

/// Generation descriptors the PPU registry held when a run measured the PPU
/// floors.
const PPU_REGISTRY_KINDS: u64 = 379;

/// Generation descriptors the SPU registry held when a run measured the SPU
/// floors.
const SPU_REGISTRY_KINDS: u64 = 194;

/// The least count of distinct instruction kinds a campaign must execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindFloor {
    /// At least this many kinds.
    AtLeast(u64),
    /// Every kind of the registry.
    EveryKind,
}

/// The relation catalog a campaign must execute in full.
///
/// The floor is the catalog's length, so it follows the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationFloor {
    /// The campaign needs no relation.
    None,
    /// Every metamorphic relation a kind of the SPU registry declares.
    InstructionCatalog,
    /// Every row of the SPU sequence-relation catalog.
    SequenceCatalog,
}

/// The least SPU facility coverage a campaign must reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacilityFloor {
    /// Distinct channels an `rdch`, `wrch` or `rchcnt` named.
    pub channels: u64,
    /// Distinct MFC command opcodes a write of `MFC_Cmd` issued.
    pub mfc_opcodes: u64,
    /// Distinct event sources that were pending.
    pub event_sources: u64,
}

/// The floor of a campaign that reaches no SPU facility.
const NO_FACILITIES: FacilityFloor = FacilityFloor {
    channels: 0,
    mfc_opcodes: 0,
    event_sources: 0,
};

/// The least coverage a smoke campaign must reach to count as a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageFloor {
    /// Generation descriptors the target's registry held when a run measured
    /// the floor.
    pub registry_kinds: u64,
    /// Cases that reached their check.
    pub eligible: u64,
    /// Distinct instruction kinds the campaign executed.
    pub instruction_kinds: KindFloor,
    /// Relation catalog the campaign must execute in full.
    pub relations: RelationFloor,
    /// SPU facilities the campaign must reach.
    pub facilities: FacilityFloor,
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
///
/// The SPU floors are the values a run at these settings reached. A run is
/// deterministic, so it reaches them exactly, and any loss fails the set.
pub const SMOKE_CAMPAIGNS: [SmokeCampaign; 8] = [
    SmokeCampaign {
        name: "ppu-instruction-structured",
        target: FuzzTarget::PpuInstruction,
        strategy: GenerationStrategy::Structured,
        seed: 1,
        cases: 300,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            registry_kinds: PPU_REGISTRY_KINDS,
            eligible: 150,
            instruction_kinds: KindFloor::AtLeast(40),
            relations: RelationFloor::None,
            facilities: NO_FACILITIES,
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
            registry_kinds: PPU_REGISTRY_KINDS,
            eligible: 150,
            instruction_kinds: KindFloor::AtLeast(40),
            relations: RelationFloor::None,
            facilities: NO_FACILITIES,
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
            registry_kinds: PPU_REGISTRY_KINDS,
            eligible: 250,
            instruction_kinds: KindFloor::AtLeast(40),
            relations: RelationFloor::None,
            facilities: NO_FACILITIES,
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
            registry_kinds: PPU_REGISTRY_KINDS,
            eligible: 250,
            instruction_kinds: KindFloor::AtLeast(40),
            relations: RelationFloor::None,
            facilities: NO_FACILITIES,
        },
    },
    // A structured case draws its kind uniformly, so the budget is the
    // least that reaches every kind and the one shufb relation.
    SmokeCampaign {
        name: "spu-instruction-structured",
        target: FuzzTarget::SpuInstruction,
        strategy: GenerationStrategy::Structured,
        seed: 1,
        cases: 1000,
        sequence_words: SMOKE_SEQUENCE_WORDS,
        floor: CoverageFloor {
            registry_kinds: SPU_REGISTRY_KINDS,
            eligible: 1000,
            instruction_kinds: KindFloor::EveryKind,
            relations: RelationFloor::InstructionCatalog,
            facilities: NO_FACILITIES,
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
            registry_kinds: SPU_REGISTRY_KINDS,
            eligible: 190,
            instruction_kinds: KindFloor::AtLeast(56),
            relations: RelationFloor::InstructionCatalog,
            facilities: NO_FACILITIES,
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
            registry_kinds: SPU_REGISTRY_KINDS,
            eligible: 252,
            instruction_kinds: KindFloor::AtLeast(167),
            relations: RelationFloor::SequenceCatalog,
            facilities: FacilityFloor {
                channels: 16,
                mfc_opcodes: 5,
                event_sources: 5,
            },
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
            registry_kinds: SPU_REGISTRY_KINDS,
            eligible: 290,
            instruction_kinds: KindFloor::AtLeast(96),
            relations: RelationFloor::SequenceCatalog,
            facilities: FacilityFloor {
                channels: 3,
                mfc_opcodes: 14,
                event_sources: 4,
            },
        },
    },
];

/// Generation descriptors in the registry `target` draws from.
fn registry_kinds(target: FuzzTarget) -> u64 {
    match target {
        FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence => ppu_descriptors().len() as u64,
        FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence => spu_descriptors().len() as u64,
    }
}

/// Kinds of `target`'s registry that `report` executed.
fn registry_kinds_reached(target: FuzzTarget, report: &FuzzReport) -> u64 {
    let identities: Vec<InstructionIdentity> = match target {
        FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence => ppu_descriptors()
            .iter()
            .map(|descriptor| InstructionIdentity::Ppu(descriptor.kind))
            .collect(),
        FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence => spu_descriptors()
            .iter()
            .map(|descriptor| InstructionIdentity::Spu(descriptor.kind))
            .collect(),
    };
    identities
        .iter()
        .filter(|identity| report.instruction_kinds.contains(identity))
        .count() as u64
}

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
    /// Names the first of:
    ///
    /// - a registry size that differs from the floor's;
    /// - a metric under the floor.
    pub fn check_coverage(&self, report: &FuzzReport) -> Result<(), SmokeError> {
        let registry = registry_kinds(self.target);
        if registry != self.floor.registry_kinds {
            return Err(SmokeError::Unpinned {
                name: self.name,
                pinned: self.floor.registry_kinds,
                registry,
            });
        }
        // `EveryKind` counts only the registry's own kinds the run reached.
        let (kinds, kinds_floor) = match self.floor.instruction_kinds {
            KindFloor::AtLeast(floor) => (report.instruction_kinds.len() as u64, floor),
            KindFloor::EveryKind => (registry_kinds_reached(self.target, report), registry),
        };
        let facilities = &report.spu_facilities;
        let floors = [
            ("eligible", report.eligible_cases, self.floor.eligible),
            ("instruction_kinds", kinds, kinds_floor),
            (
                "channels",
                facilities.channels.len() as u64,
                self.floor.facilities.channels,
            ),
            (
                "mfc_opcodes",
                facilities.mfc_opcodes.len() as u64,
                self.floor.facilities.mfc_opcodes,
            ),
            (
                "event_sources",
                facilities.event_sources.len() as u64,
                self.floor.facilities.event_sources,
            ),
        ];
        for (metric, found, floor) in floors {
            vacuous_under(self.name, metric, found, floor)?;
        }
        let (metric, catalog): (_, Vec<CheckIdentity>) = match self.floor.relations {
            RelationFloor::None => return Ok(()),
            RelationFloor::InstructionCatalog => (
                "instruction_relations",
                crate::spu::declared_relation_checks().into_iter().collect(),
            ),
            RelationFloor::SequenceCatalog => (
                "sequence_relation_rows",
                sequence_relations()
                    .iter()
                    .map(|row| CheckIdentity::SpuSequenceRelation(row.id))
                    .collect(),
            ),
        };
        let executed = catalog
            .iter()
            .filter(|check| {
                report
                    .metamorphic_executions
                    .get(check)
                    .is_some_and(|&count| count > 0)
            })
            .count() as u64;
        vacuous_under(self.name, metric, executed, catalog.len() as u64)
    }
}

fn vacuous_under(
    name: &'static str,
    metric: &'static str,
    found: u64,
    floor: u64,
) -> Result<(), SmokeError> {
    if found < floor {
        return Err(SmokeError::Vacuous {
            name,
            metric,
            found,
            floor,
        });
    }
    Ok(())
}

/// The reference tier of the smoke set: every committed SPU reference
/// file, replayed against the interpreter and the scripted world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmokeReference {
    /// Name the set, the command output, and the tests use.
    pub name: &'static str,
    /// Units a reference could cover when a run measured the floor.
    pub units: u64,
    /// Files the directory must hold.
    pub files: u64,
    /// Vectors the files must replay.
    pub vectors: u64,
}

/// The reference tier, with its floor measured on the committed fixtures.
pub const SMOKE_REFERENCE: SmokeReference = SmokeReference {
    name: "spu-reference-replay",
    units: 276,
    files: 276,
    vectors: 1542,
};

impl SmokeReference {
    /// Replays every reference file in `dir`.
    ///
    /// # Errors
    ///
    /// A directory or file the replay cannot read.
    pub fn run(&self, dir: &Path) -> Result<SpuReferenceCampaign, SpuReferenceCampaignError> {
        run_reference_directory(dir)
    }

    /// Checks that a replay matched, is complete, and reached the floor.
    ///
    /// # Errors
    ///
    /// Names the first of:
    ///
    /// - a unit count that differs from the floor's;
    /// - a file that did not match;
    /// - a gap in the directory;
    /// - a metric under the floor.
    pub fn check(&self, campaign: &SpuReferenceCampaign) -> Result<(), SmokeError> {
        let units = all_units().len() as u64;
        if units != self.units {
            return Err(SmokeError::Unpinned {
                name: self.name,
                pinned: self.units,
                registry: units,
            });
        }
        if let Some(run) = campaign.files.iter().find(|run| !run.outcome.is_match()) {
            return Err(SmokeError::ReferenceMismatch {
                name: self.name,
                file: run.file.clone(),
            });
        }
        let completeness = &campaign.completeness;
        if !completeness.is_complete() {
            return Err(SmokeError::ReferenceIncomplete {
                name: self.name,
                gaps: completeness.gaps(),
            });
        }
        vacuous_under(self.name, "files", campaign.files.len() as u64, self.files)?;
        vacuous_under(
            self.name,
            "vectors",
            campaign.vectors() as u64,
            self.vectors,
        )
    }

    /// Whether the replay holds the floor's files and vectors, matched or not.
    pub fn reaches_floor(&self, campaign: &SpuReferenceCampaign) -> bool {
        campaign.files.len() as u64 >= self.files && campaign.vectors() as u64 >= self.vectors
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
    /// The registry size differs from the one the floor names.
    #[error(
        "smoke campaign {name} has a floor for a registry of {pinned} entries, \
         which now holds {registry}; measure the floor again"
    )]
    Unpinned {
        /// Campaign name.
        name: &'static str,
        /// Registry size the floor names.
        pinned: u64,
        /// Registry size now.
        registry: u64,
    },
    /// A reference file did not replay to its expected observation.
    #[error("smoke campaign {name}: {file} does not match its reference")]
    ReferenceMismatch {
        /// Campaign name.
        name: &'static str,
        /// The first file that did not match.
        file: String,
    },
    /// The reference directory has a gap.
    ///
    /// A gap is one of:
    ///
    /// - a unit with no file and no pending entry;
    /// - a unit with more than one file;
    /// - a file or pending entry that names no unit;
    /// - a pending entry whose unit has a file;
    /// - a pending entry the list names twice.
    #[error("smoke campaign {name}: the reference directory is incomplete ({gaps} gaps)")]
    ReferenceIncomplete {
        /// Campaign name.
        name: &'static str,
        /// Gaps, as [`crate::spu_reference::SpuReferenceCompleteness::gaps`]
        /// counts them.
        gaps: usize,
    },
}

#[cfg(test)]
#[path = "tests/smoke_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/smoke_tier_tests.rs"]
mod tier_tests;
