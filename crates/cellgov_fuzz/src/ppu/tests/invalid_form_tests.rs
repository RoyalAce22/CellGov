use super::*;
use cellgov_ppu::instruction::fuzz::{PpuFuzzKind, PpuOutcomeClass};
use cellgov_ppu::instruction::PpuInstructionKind;

use crate::campaign::{CampaignSchedule, CaseRange};

#[test]
fn an_update_form_in_an_invalid_form_is_no_finding() {
    for (index, kind) in [
        (10_070, PpuInstructionKind::Stbu),
        (10_065, PpuInstructionKind::Lhzu),
        (101_835, PpuInstructionKind::Lfsu),
        (953_273, PpuInstructionKind::Stfdux),
    ] {
        let run = run_instructions(FuzzConfig {
            seed: 11,
            strategy: GenerationStrategy::RawWords,
            schedule: CampaignSchedule {
                cases: CaseRange {
                    first: index,
                    count: 1,
                },
                ..CampaignSchedule::default()
            },
            ..FuzzConfig::default()
        });
        assert_eq!(
            run.report.instruction_kinds,
            BTreeSet::from([InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(kind))]),
            "case {index}"
        );
        assert_eq!(run.report.eligible_cases, 1, "case {index}");
        // No write reaches the commit.
        assert!(
            run.report.effect_classes.is_empty(),
            "case {index}: {:?}",
            run.report.effect_classes
        );
        assert!(
            run.report.finding_counts.is_empty(),
            "case {index}: {:?}",
            run.report.finding_counts
        );
    }
}

#[test]
fn only_an_invalid_update_encoding_admits_a_fault() {
    let outcomes = |raw: u32| {
        cellgov_ppu::decode::decode(raw)
            .unwrap()
            .fuzz_descriptor(raw)
            .outcomes
    };
    for (invalid, valid) in [
        // stwu r3,16(r0) / stwu r3,16(r1)
        (
            (37 << 26) | (3 << 21) | 16,
            (37 << 26) | (3 << 21) | (1 << 16) | 16,
        ),
        // lhzu r4,0(r4) / lhzu r4,0(r5)
        (
            (41 << 26) | (4 << 21) | (4 << 16),
            (41 << 26) | (4 << 21) | (5 << 16),
        ),
        // lfsu f1,0(r0) / lfsu f1,0(r1): RA=FRT is a valid form.
        ((49 << 26) | (1 << 21), (49 << 26) | (1 << 21) | (1 << 16)),
    ] {
        assert_eq!(
            outcomes(invalid),
            &[PpuOutcomeClass::Fault],
            "{invalid:#010x}"
        );
        assert!(
            !outcomes(valid).contains(&PpuOutcomeClass::Fault),
            "{valid:#010x}: {:?}",
            outcomes(valid)
        );
    }
}
