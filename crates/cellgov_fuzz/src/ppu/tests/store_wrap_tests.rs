use super::*;
use cellgov_ppu::instruction::fuzz::PpuFuzzKind;
use cellgov_ppu::instruction::PpuInstructionKind;

use crate::campaign::{CampaignSchedule, CaseRange};

fn case(strategy: GenerationStrategy, index: u64) -> FuzzConfig {
    FuzzConfig {
        seed: 11,
        strategy,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: index,
                count: 1,
            },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    }
}

#[test]
fn a_store_that_wraps_the_address_space_is_no_finding() {
    for (strategy, index, kind) in [
        (GenerationStrategy::Structured, 83, PpuInstructionKind::Stw),
        (
            GenerationStrategy::Structured,
            113_402,
            PpuInstructionKind::Stmw,
        ),
        (
            GenerationStrategy::RawWords,
            5_553_226,
            PpuInstructionKind::Dcbz,
        ),
    ] {
        let run = run_instructions(case(strategy, index));
        assert_eq!(run.report.cases, 1, "case {index}");
        assert_eq!(
            run.report.instruction_kinds,
            BTreeSet::from([InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(kind))]),
            "case {index}"
        );
        // The store faulted: no write effect reached the commit.
        assert!(
            run.report.effect_classes.is_empty(),
            "case {index}: {:?}",
            run.report.effect_classes
        );
        // A structured case that faults did not meet its mapped-memory precondition.
        let (eligible, unsupported) = match strategy {
            GenerationStrategy::Structured => (0, 1),
            GenerationStrategy::RawWords => (1, 0),
        };
        assert_eq!(run.report.eligible_cases, eligible, "case {index}");
        assert_eq!(run.report.unsupported_cases, unsupported, "case {index}");
        assert!(
            run.report.finding_counts.is_empty(),
            "case {index}: {:?}",
            run.report.finding_counts
        );
    }
}
