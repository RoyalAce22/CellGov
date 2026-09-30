use super::*;

use crate::report::{CheckIdentity, FindingKind};
use crate::smoke::SMOKE_CAMPAIGNS;
use crate::FuzzTarget;

const SEQUENCE_DEFECTS: [SeededDefect; 2] = [
    SeededDefect::SequencePartnerWrite,
    SeededDefect::SequencePartnerLane,
];

#[test]
fn each_sequence_partner_defect_is_caught_by_a_sequence_relation_and_nothing_else() {
    for defect in SEQUENCE_DEFECTS {
        let _guard = seed(defect);
        for campaign in SMOKE_CAMPAIGNS
            .iter()
            .filter(|campaign| campaign.target == FuzzTarget::SpuSequence)
        {
            let run = campaign.run();
            let report = &run.report;
            assert!(
                report
                    .finding_counts
                    .get(&FindingKind::MetamorphicViolation)
                    .is_some_and(|&count| count > 0),
                "{defect:?} {}: no relation caught it",
                campaign.name
            );
            assert_eq!(
                report.finding_counts.keys().collect::<Vec<_>>(),
                [&FindingKind::MetamorphicViolation],
                "{defect:?} {}: another check fired",
                campaign.name
            );
            for finding in &report.findings {
                assert!(
                    matches!(
                        finding.fingerprint.check,
                        CheckIdentity::SpuSequenceRelation(_)
                    ),
                    "{defect:?} {}: {:?}",
                    campaign.name,
                    finding.fingerprint.check
                );
            }
            assert_eq!(
                report.relation_counterexamples.len(),
                report.findings.len(),
                "{defect:?} {}: every retained relation finding keeps a fixture",
                campaign.name
            );
        }
    }
}

#[test]
fn a_seeded_sequence_campaign_replays_the_stored_counterexample_before_its_first_case() {
    let _guard = seed(SeededDefect::SequencePartnerWrite);
    let campaign = SMOKE_CAMPAIGNS
        .iter()
        .find(|campaign| campaign.target == FuzzTarget::SpuSequence)
        .expect("the smoke set has an SPU sequence campaign");
    let run = campaign.run();
    let replay = run
        .report
        .stored_replays
        .iter()
        .find(|replay| replay.name == "seeded-partner-write")
        .expect("the campaign replayed the store");
    assert!(replay.divergence.is_some());
    // A range that does not hold case 0 leaves the store to the run that does.
    let mut config = campaign.config();
    config.schedule.cases.first = 1;
    config.schedule.cases.count = 2;
    assert!(crate::spu::run_sequences(config)
        .report
        .stored_replays
        .is_empty());
}

#[test]
fn the_instruction_tiers_miss_every_sequence_partner_defect() {
    for defect in SEQUENCE_DEFECTS {
        let _guard = seed(defect);
        for campaign in SMOKE_CAMPAIGNS
            .iter()
            .filter(|campaign| campaign.target == FuzzTarget::SpuInstruction)
        {
            let run = campaign.run();
            assert!(
                run.report.is_clean(),
                "{defect:?} {}: {:?}",
                campaign.name,
                run.report.finding_counts
            );
            assert!(run.report.eligible_cases > 0, "{}", campaign.name);
        }
    }
}
