//! Each SPU metamorphic relation catches a seeded defect that every other
//! check misses: the replay, the outcome, effect and footprint contracts,
//! and the word's other relations.

use super::*;

use cellgov_spu::fuzz::generation_descriptors;
use cellgov_spu::instruction::SpuInstructionKind;

use crate::reduce::evaluate_case;
use crate::report::{CheckIdentity, FindingKind};
use crate::{
    CampaignSchedule, CampaignShard, CaseRange, FuzzConfig, FuzzTarget, GenerationStrategy,
};

/// Cases each word runs from; each case draws its own initial state.
const CASES: u64 = 8;

fn config() -> FuzzConfig {
    FuzzConfig {
        seed: 11,
        strategy: GenerationStrategy::Structured,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: CASES,
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        },
        max_findings: 64,
        ..FuzzConfig::default()
    }
}

/// `kind`'s word with every operand field zero, OR `fields`.
fn word(kind: SpuInstructionKind, fields: u32) -> u32 {
    generation_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.kind == kind)
        .expect("every kind has a descriptor")
        .canonical_word
        | fields
}

/// RT 3, RA 4, RB 5.
const RR: u32 = 5 << 14 | 4 << 7 | 3;

fn cases() -> [(SeededDefect, u32, CheckIdentity); 6] {
    [
        // stop 0x2000 with its ignored bits clear; the partner sets them.
        (
            SeededDefect::IgnoredFieldRead,
            word(SpuInstructionKind::Stop, 0x2000),
            CheckIdentity::SpuIgnoredField,
        ),
        // rotqbyi rt3, ra4, 3: the partner sets the masked-off count bits.
        (
            SeededDefect::UnmaskedCount,
            word(SpuInstructionKind::Rotqbyi, 3 << 14 | 4 << 7 | 3),
            CheckIdentity::SpuCountMasking,
        ),
        // ai rt3, ra4, 5: the partner is `a` with 5 in every word of RB.
        (
            SeededDefect::ImmediateFormOnly,
            word(SpuInstructionKind::Ai, 5 << 14 | 4 << 7 | 3),
            CheckIdentity::SpuImmediateRegister,
        ),
        // and rt3, ra4, rb5: only the partner has RA above RB.
        (
            SeededDefect::OperandOrder,
            word(SpuInstructionKind::And, RR),
            CheckIdentity::SpuCommutative,
        ),
        // sf rt3, ra4, rb5: the partner's first slot comes back as byte 8.
        (
            SeededDefect::FirstSlot,
            word(SpuInstructionKind::Sf, RR),
            CheckIdentity::SpuSlotPermutation,
        ),
        // brz rt3, +4 words: not taken on a nonzero word; the partner is
        // brnz on a zero mask, also not taken.
        (
            SeededDefect::BranchFallThrough,
            word(SpuInstructionKind::Brz, 4 << 7 | 3),
            CheckIdentity::SpuCompareBranch,
        ),
    ]
}

#[test]
fn each_relation_catches_its_seeded_defect_and_no_other_check_does() {
    for (defect, raw, check) in cases() {
        let words = [raw];
        for index in 0..CASES {
            let clean = evaluate_case(FuzzTarget::SpuInstruction, config(), index, &words);
            assert!(
                clean.report.is_clean(),
                "{defect:?} case {index}: clean run found {:?}",
                clean.report.findings
            );
            assert!(
                clean
                    .report
                    .metamorphic_executions
                    .get(&check)
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "{defect:?} case {index}: {check:?} did not execute: {:?}",
                clean.report.metamorphic_executions
            );

            let _guard = seed(defect);
            let seeded = evaluate_case(FuzzTarget::SpuInstruction, config(), index, &words);
            assert!(
                !seeded.report.findings.is_empty(),
                "{defect:?} case {index}: the seeded defect went unseen"
            );
            for finding in &seeded.report.findings {
                assert_eq!(
                    (finding.kind, finding.fingerprint.check),
                    (FindingKind::MetamorphicViolation, check),
                    "{defect:?} case {index}: {finding:?}"
                );
            }
        }
    }
}
