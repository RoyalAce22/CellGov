//! Each PPU tier of the smoke set reaches its floor, fails it when a
//! metric drops, and catches a defect seeded into it at its budget.

use super::*;

use std::collections::BTreeSet;

use crate::report::FindingKind;
use crate::seeded::{seed, SeededDefect};

fn smoke(name: &str) -> SmokeCampaign {
    *SMOKE_CAMPAIGNS
        .iter()
        .find(|campaign| campaign.name == name)
        .unwrap_or_else(|| panic!("the smoke set has {name}"))
}

#[test]
fn the_structured_ppu_instruction_campaign_needs_every_kind_and_every_declared_relation() {
    assert_eq!(
        crate::ppu::declared_relation_checks(),
        BTreeSet::from([
            CheckIdentity::PpuRecordCr0,
            CheckIdentity::PpuRecordCr1,
            CheckIdentity::PpuRecordCr6,
            CheckIdentity::PpuOverflowEnable,
        ])
    );
    let campaign = smoke("ppu-instruction-structured");
    let mut report = campaign.run().report;
    campaign
        .check_coverage(&report)
        .unwrap_or_else(|error| panic!("{error}"));
    // A kind no case runs is under the floor: a kind the registry gains
    // fails the set until a case reaches it.
    let kind = *report.instruction_kinds.first().expect("a kind ran");
    report.instruction_kinds.remove(&kind);
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            metric: "instruction_kinds",
            found,
            floor,
            ..
        }) if found + 1 == floor && floor == PPU_REGISTRY_KINDS
    ));
    // A kind outside the registry does not count toward the registry's kinds.
    report
        .instruction_kinds
        .insert(InstructionIdentity::Spu(spu_descriptors()[0].kind));
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            metric: "instruction_kinds",
            ..
        })
    ));
    report.instruction_kinds.insert(kind);
    report
        .metamorphic_executions
        .remove(&CheckIdentity::PpuRecordCr6);
    assert!(matches!(
        campaign.check_coverage(&report),
        Err(SmokeError::Vacuous {
            metric: "instruction_relations",
            found: 3,
            floor: 4,
            ..
        })
    ));
}

#[test]
fn each_ppu_relation_defect_is_caught_by_the_structured_instruction_campaign_at_its_budget() {
    let campaign = smoke("ppu-instruction-structured");
    for (defect, check) in [
        (SeededDefect::PpuRecordFormGpr, CheckIdentity::PpuRecordCr0),
        (SeededDefect::PpuRecordFormFpr, CheckIdentity::PpuRecordCr1),
        (SeededDefect::PpuRecordFormVr, CheckIdentity::PpuRecordCr6),
        (
            SeededDefect::PpuOverflowFormGpr,
            CheckIdentity::PpuOverflowEnable,
        ),
    ] {
        let _guard = seed(defect);
        let report = campaign.run().report;
        let checks: BTreeSet<CheckIdentity> = report
            .findings
            .iter()
            .map(|finding| finding.fingerprint.check)
            .collect();
        assert_eq!(checks, BTreeSet::from([check]), "{defect:?}");
        assert_eq!(
            report.finding_counts.keys().collect::<Vec<_>>(),
            [&FindingKind::MetamorphicViolation],
            "{defect:?}"
        );
    }
}

#[test]
fn the_raw_word_instruction_campaign_catches_an_outcome_and_an_effect_outside_the_contract() {
    let campaign = smoke("ppu-instruction-raw-words");
    for (defect, kind) in [
        (SeededDefect::IllegalOutcome, FindingKind::IllegalOutcome),
        (SeededDefect::IllegalEffect, FindingKind::IllegalEffect),
    ] {
        let _guard = seed(defect);
        let report = campaign.run().report;
        assert_eq!(
            report.finding_counts.keys().collect::<Vec<_>>(),
            [&kind],
            "{defect:?}"
        );
    }
}

#[test]
fn the_sequence_campaigns_check_replay_and_no_relation() {
    for name in ["ppu-sequence-structured", "ppu-sequence-raw-words"] {
        let campaign = smoke(name);
        {
            let _guard = seed(SeededDefect::Nondeterministic);
            let report = campaign.run().report;
            assert_eq!(
                report.finding_counts.keys().collect::<Vec<_>>(),
                [&FindingKind::Nondeterministic],
                "{name}"
            );
        }
        // Both runs of a sequence carry the same wrong value, so a
        // relation defect leaves the replay clean.
        let _guard = seed(SeededDefect::PpuRecordFormFpr);
        let report = campaign.run().report;
        assert!(report.is_clean(), "{name}: {:?}", report.finding_counts);
    }
}
