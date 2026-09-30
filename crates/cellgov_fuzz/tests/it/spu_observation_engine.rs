//! SPU fuzz footprint conformance checks.

use cellgov_fuzz::{
    spu, CampaignSchedule, CaseRange, CheckIdentity, FindingKind, FuzzConfig, RunOutcome,
};

#[test]
fn structured_spu_cases_use_the_complete_footprint_without_false_findings() {
    let run = spu::run_instructions(FuzzConfig {
        seed: 17,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 2_048,
            },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    });

    assert_eq!(run.report.cases, 2_048);
    assert_eq!(run.report.decoded, 2_048);
    assert!(run.report.eligible_cases > 0);
    assert_eq!(run.outcome, RunOutcome::CleanCompletion);
    assert_eq!(
        run.report
            .finding_counts
            .get(&FindingKind::IllegalFootprint),
        None
    );
    assert!(!run
        .report
        .findings
        .iter()
        .any(|finding| { finding.fingerprint.check == CheckIdentity::AllowedFootprint }));
}
